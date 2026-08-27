# 对话面板导航改版(session 列表 → session 详情) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把对话面板从"当前项目全部 session 的回合拍平成一份按时间倒序列表"改成"session 列表(标题+总结预览)→点进去看该 session 的标题/总结全文/整段对话式回合列表",彻底替代现有扁平列表,不做视图共存。

**Architecture:** `dozerd` 新增一个跨 store 联查(`ListConversationsWithSummaries`:先查 `conversations` 表拿会话列表,再用得到的 `conversation_id` 批量查 `session_summaries` 表,Rust 侧拼成 `Vec<(ConversationSummary, Option<SessionSummaryPayload>)>`);`dozer-app` 侧数据类型/渲染/详情加载全部换成"session 粒度",不再有"回合区间"这个概念;`ListAllTurnGroups`/`ListSessionTurnGroups` 这套 8/21 拍平前后遗留的协议+代码作为死代码一并删除。

**Tech Stack:** Rust workspace(iced 0.14 / tokio / rusqlite / dozer-core protocol),UDS + JSON Lines 协议。

**Spec:** `docs/superpowers/specs/2026-08-27-conversation-panel-session-navigation-design.md`

**硬依赖(已满足)**:`session_summaries` 表的 `conversation_id` 列(姊妹计划 `docs/superpowers/plans/2026-08-27-session-summary-pipeline.md` 的"⚠️ 修正"章节)已在 commit `c695b44` 落地并验证通过——`SessionSummaryStore` 现有 `record`/`get` 均已带 `conversation_id: Option<String>`,本计划的 `get_many` 直接建立在这个字段之上。

## Global Constraints

- 在独立分支上开发(建议延续 `feature/session-summary-pipeline` 分支,因为本计划硬依赖该分支上已落地的 `conversation_id` 列;若另开新分支,必须先 merge/rebase 那个修复),不要直接提交到 main;全部 Task 完成、构建/测试/clippy/fmt 全绿后提请审阅,审阅通过再合并。
- 彻底替代现有扁平回合列表,不做视图共存开关。
- 没有总结的 session(旧数据/纯 shell/SSH/Codex/Kilo)降级显示 `ConversationSummary.title`,不加"未总结"标记。
- `prev_topic`/`next_topic`/`TopicPreview`/`adjacent_topic_previews` 整体删除,改为点列表切换。
- `SessionRow`/`ReviewView` 存总结**全文**(不预先截断),列表行渲染时才截断成预览;详情页直接用全文,不为详情页单独发一次 `get_session_summary` 请求。
- 死代码清理范围:`Request::ListAllTurnGroups`/`Reply::AllTurnGroups`/`TurnGroupEntry`/`Request::ListSessionTurnGroups`/`Reply::SessionTurnGroups`/`TurnGroupSummary`(协议层)、`TranscriptStore::list_all_turn_groups`/`list_all_turn_groups_in`/`list_turn_groups`(dozerd)、`Client::list_all_turn_groups`/`list_session_turn_groups`(dozer-client)、`conversation.rs::TurnGroupRow`/`from_entry`(dozer-app)、`ReviewSource::FileRange`(dozer-app,详情见 Task 7)。
- 每个改动完成后运行:`cargo build`(全 workspace)、对应 crate 的 `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt -- --check`。

---

## Task 1: 协议层——新增联查请求,删除死协议

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces: `Request::ListConversationsWithSummaries { cwd: String, agent: Option<AgentKind>, limit: u32, offset: u32 }` → `Reply::ConversationsWithSummaries { rows: Vec<(ConversationSummary, Option<SessionSummaryPayload>)> }`。后续 Task 3(dozerd handler)/Task 4(dozer-client 方法)直接消费这两个类型。

- [ ] **Step 1: 写失败的序列化往返测试**

在 `protocol.rs` 的 `#[cfg(test)] mod tests` 块内追加:

```rust
    #[test]
    fn list_conversations_with_summaries_request_roundtrips() {
        let req = Request::ListConversationsWithSummaries {
            cwd: "/tmp".into(),
            agent: Some(AgentKind::Claude),
            limit: 50,
            offset: 0,
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn conversations_with_summaries_reply_roundtrips() {
        let reply = Reply::ConversationsWithSummaries {
            rows: vec![
                (
                    ConversationSummary {
                        conversation_id: "c1".into(),
                        agent: AgentKind::Claude,
                        file_path: "/h/.claude/projects/x/c1.jsonl".into(),
                        title: "标题".into(),
                        first_ts: 1,
                        last_ts: 2,
                        turn_count: 3,
                    },
                    Some(SessionSummaryPayload {
                        session_id: "s1".into(),
                        agent_kind: AgentKind::Claude,
                        conversation_id: Some("c1".into()),
                        title: "总结标题".into(),
                        summary: "总结全文".into(),
                        status: SummaryStatus::AiGenerated,
                        created_ts_ms: 42,
                    }),
                ),
                (
                    ConversationSummary {
                        conversation_id: "c2".into(),
                        agent: AgentKind::Codex,
                        file_path: "/h/.codex/x/c2.jsonl".into(),
                        title: "无总结的旧会话".into(),
                        first_ts: 1,
                        last_ts: 2,
                        turn_count: 1,
                    },
                    None,
                ),
            ],
        };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core with_summaries`
Expected: 编译失败,`ListConversationsWithSummaries`/`ConversationsWithSummaries` 未定义。

- [ ] **Step 3: 加新 variant,删除死 variant/类型**

在 `pub enum Request` 里,`GetUsageSummary { .. }` 之后追加:

```rust
    /// 某 cwd 下的会话列表,联查上 `session_summaries`(有则 `Some`,历史/
    /// 未支持 agent 类型的会话为 `None`)。取代 2026-08-21 引入、现已死的
    /// `ListAllTurnGroups`(spec 2026-08-27)。
    ListConversationsWithSummaries {
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    },
```

删除 `Request` 里的 `ListAllTurnGroups { cwd: String, limit: u32 }` 和
`ListSessionTurnGroups { conversation_id: String }` 两个 variant(连同各自的文档注释)。

在 `pub enum Reply` 里,`UsageSummary { .. }` 之后追加:

```rust
    /// `ListConversationsWithSummaries` 应答。
    ConversationsWithSummaries {
        rows: Vec<(ConversationSummary, Option<SessionSummaryPayload>)>,
    },
```

删除 `Reply` 里的 `AllTurnGroups { groups: Vec<TurnGroupEntry> }` 和
`SessionTurnGroups { conversation_id: String, groups: Vec<TurnGroupSummary> }` 两个 variant。

删除类型定义:`pub struct TurnGroupSummary`、`pub struct TurnGroupEntry`(连同各自上方文档注释)。

删除 `#[cfg(test)] mod tests` 里所有引用了 `ListAllTurnGroups`/`AllTurnGroups`/
`TurnGroupEntry`/`ListSessionTurnGroups`/`SessionTurnGroups`/`TurnGroupSummary` 的测试函数(用 `grep -n "TurnGroup" crates/dozer-core/src/protocol.rs` 定位,逐个确认是纯往返测试后删除;若某个测试同时覆盖了别的东西,只删测试体里跟这几个类型相关的部分,不要整个函数一起删)。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core`
Expected: 全绿(新增 2 个测试通过,删除的类型不再有任何编译引用)。

- [ ] **Step 5: clippy + fmt**

Run: `cargo clippy -p dozer-core --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): add ListConversationsWithSummaries, remove dead TurnGroup protocol"
```

---

## Task 2: `SessionSummaryStore::get_many` 批量查询

**Files:**
- Modify: `crates/dozerd/src/session_summary.rs`

**Interfaces:**
- Consumes: 现有 `SessionSummaryStore`(已含 `conversation_id` 列,commit `c695b44`)。
- Produces: `pub fn get_many(&self, conversation_ids: &[String]) -> Result<HashMap<String, SessionSummaryPayload>>`。Task 3 的联查 handler 直接调用。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozerd/src/session_summary.rs` 的 `#[cfg(test)] mod tests` 块内追加:

```rust
    #[test]
    fn get_many_empty_input_returns_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.get_many(&[]).unwrap().is_empty());
    }

    #[test]
    fn get_many_batches_multiple_ids_partial_hit() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p1 = payload("s1");
        p1.conversation_id = Some("c1".into());
        let mut p2 = payload("s2");
        p2.conversation_id = Some("c2".into());
        store.record(&p1).unwrap();
        store.record(&p2).unwrap();

        let got = store
            .get_many(&["c1".to_string(), "c2".to_string(), "c-missing".to_string()])
            .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got["c1"].session_id, "s1");
        assert_eq!(got["c2"].session_id, "s2");
        assert!(!got.contains_key("c-missing"));
    }

    #[test]
    fn get_many_ignores_rows_with_null_conversation_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        p.conversation_id = None;
        store.record(&p).unwrap();
        // 查一个跟这行完全无关的 id 列表——不该因为库里存在 conversation_id
        // 为 NULL 的行就出错或误命中。
        assert!(store.get_many(&["c1".to_string()]).unwrap().is_empty());
    }

    #[test]
    fn get_many_ids_with_special_characters_are_parameter_bound_not_concatenated() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        // 含单引号的 id——若实现拼字符串而不是走参数绑定,这里会破坏 SQL。
        p.conversation_id = Some("weird'id".into());
        store.record(&p).unwrap();
        let got = store.get_many(&["weird'id".to_string()]).unwrap();
        assert_eq!(got.len(), 1);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd get_many`
Expected: 编译失败,`get_many` 未定义。

- [ ] **Step 3: 实现**

在 `crates/dozerd/src/session_summary.rs` 顶部 `use` 列表追加 `use std::collections::HashMap;`,在 `impl SessionSummaryStore` 的 `get` 方法之后追加:

```rust
    pub fn get_many(
        &self,
        conversation_ids: &[String],
    ) -> Result<HashMap<String, SessionSummaryPayload>> {
        if conversation_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = conversation_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms
             FROM session_summaries WHERE conversation_id IN ({placeholders})"
        );
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(conversation_ids.iter()), |row| {
            let agent_kind: String = row.get(1)?;
            let conversation_id: Option<String> = row.get(2)?;
            let status: String = row.get(5)?;
            let created_ts_ms: i64 = row.get(6)?;
            Ok((
                conversation_id.clone(),
                SessionSummaryPayload {
                    session_id: row.get(0)?,
                    agent_kind: agent_from_str(&agent_kind),
                    conversation_id,
                    title: row.get(3)?,
                    summary: row.get(4)?,
                    status: status_from_str(&status),
                    created_ts_ms: created_ts_ms as u64,
                },
            ))
        })?;
        let mut out = HashMap::new();
        for r in rows {
            let (conversation_id, payload) = r?;
            if let Some(cid) = conversation_id {
                out.insert(cid, payload);
            }
        }
        Ok(out)
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd get_many`
Expected: 4 个测试全 PASS。

- [ ] **Step 5: clippy + fmt**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/session_summary.rs
git commit -m "feat(dozerd): add SessionSummaryStore::get_many batch lookup"
```

---

## Task 3: `dozerd` 联查 handler,删除死查询

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Consumes: `TranscriptStore::list_conversations`(已有)、`SessionSummaryStore::get_many`(Task 2)、`Request::ListConversationsWithSummaries`/`Reply::ConversationsWithSummaries`(Task 1)。
- Produces: `handle_conn` 新增该 Request 的处理分支。

- [ ] **Step 1: 写失败的集成测试**

在 `crates/dozerd/tests/hook_events.rs` 末尾追加(复用文件里已有的 `test_store`/`test_projects`/`test_bookmarks`/`test_transcripts`/`test_session_summaries` helper 和 serve 启动套路,参照文件内其余测试的写法):

```rust
#[tokio::test]
async fn list_conversations_with_summaries_joins_correctly() {
    let sock = std::env::temp_dir().join(format!("dozerd-join-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let transcripts = test_transcripts();
    let session_summaries = test_session_summaries();
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        let transcripts = transcripts.clone();
        let session_summaries = session_summaries.clone();
        async move {
            dozerd::server::serve(
                &sock, registry, test_store(), test_projects(), test_bookmarks(),
                transcripts, session_summaries,
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // 造一个真实摄取的 conversation(c1)+一个匹配的 session_summaries 行,
    // 另一个 conversation(c2)不给总结,验证降级为 None。
    let dir = std::env::temp_dir().join(format!("dozerd-join-cwd-{}", uuid::Uuid::new_v4()));
    let claude_dir = dir.join(".claude").join("projects").join("x");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("c1.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"c1 内容\"}}\n",
    )
    .unwrap();
    std::fs::write(
        claude_dir.join("c2.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"c2 内容\"}}\n",
    )
    .unwrap();
    transcripts
        .ingest_session(
            dozer_core::protocol::AgentKind::Claude,
            &claude_dir.join("c1.jsonl"),
        )
        .unwrap();
    transcripts
        .ingest_session(
            dozer_core::protocol::AgentKind::Claude,
            &claude_dir.join("c2.jsonl"),
        )
        .unwrap();
    session_summaries
        .record(&dozer_core::protocol::SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: dozer_core::protocol::AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: "c1 的总结标题".into(),
            summary: "c1 的总结全文".into(),
            status: dozer_core::protocol::SummaryStatus::AiGenerated,
            created_ts_ms: 1,
        })
        .unwrap();

    let client = dozer_client::Client::new(sock);
    let rows = client
        .list_conversations_with_summaries(&dir.to_string_lossy(), None, 50, 0)
        .await
        .unwrap();

    assert_eq!(rows.len(), 2);
    let c1 = rows
        .iter()
        .find(|(c, _)| c.conversation_id == "c1")
        .unwrap();
    assert_eq!(c1.1.as_ref().unwrap().title, "c1 的总结标题");
    let c2 = rows
        .iter()
        .find(|(c, _)| c.conversation_id == "c2")
        .unwrap();
    assert!(c2.1.is_none());
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd --test hook_events list_conversations_with_summaries_joins_correctly`
Expected: 编译失败(`Client::list_conversations_with_summaries` 还不存在——这个方法在 Task 4 才加;先跑这一步会在 Task 4 完成前一直失败是预期的,记录下来,等 Task 4 落地后回来跑通即可。若你希望本任务能独立跑通测试,可以改用裸 `Request`/`Reply` 直接发协议消息,不经过 `Client` 封装,参照 `session_survival.rs` 里 `struct Client`(裸 UDS 收发)的写法自己在这个测试里写一份等价逻辑——两种做法选一种,不要两者都做)。

- [ ] **Step 3: 实现 handler**

在 `crates/dozerd/src/server.rs` 的 `Request::GetUsageSummary { .. }` 分支之后追加:

```rust
                        Request::ListConversationsWithSummaries { cwd, agent, limit, offset } => {
                            match transcripts.list_conversations(&cwd, agent, limit, offset) {
                                Ok(conversations) => {
                                    let ids: Vec<String> = conversations
                                        .iter()
                                        .map(|c| c.conversation_id.clone())
                                        .collect();
                                    let summaries =
                                        session_summaries.get_many(&ids).unwrap_or_default();
                                    let rows = conversations
                                        .into_iter()
                                        .map(|c| {
                                            let s = summaries.get(&c.conversation_id).cloned();
                                            (c, s)
                                        })
                                        .collect();
                                    Reply::ConversationsWithSummaries { rows }
                                }
                                Err(e) => Reply::Error { message: format!("列对话失败: {e}") },
                            }
                        }
```

删除 `Request::ListAllTurnGroups { cwd, limit } => { ... }` 和
`Request::ListSessionTurnGroups { conversation_id } => { ... }` 两个分支。

- [ ] **Step 4: 删除 `TranscriptStore` 里的死查询**

在 `crates/dozerd/src/transcripts/mod.rs` 里删除
`list_all_turn_groups`/`list_all_turn_groups_in`/`list_turn_groups` 三个方法
(用 `grep -n "fn list_all_turn_groups\|fn list_turn_groups" crates/dozerd/src/transcripts/mod.rs` 定位),以及对应的单测(`list_all_turn_groups_falls_back_to_session_last_ts_when_group_ts_is_zero`/
`list_all_turn_groups_flattens_across_sessions_sorted_by_ts_desc`/
`list_turn_groups_on_unknown_conversation_returns_empty`/
`list_turn_groups_splits_on_real_human_turns_and_folds_commands`,用
`cargo test -p dozerd 2>&1 | grep "list_.*turn_group"` 跑一遍现状确认测试名单跟这里列的一致,再逐个删)。

- [ ] **Step 5: 运行测试确认通过(先跳过依赖 Task 4 的那条)**

Run: `cargo test -p dozerd --lib`
Expected: 全绿(dozerd 单测不依赖 dozer-client,应该已经能跑通;`--test hook_events` 里新加的那条会在 Task 4 完成后才能编译过,这一步先不强求)。

- [ ] **Step 6: clippy + fmt(dozerd 库本身)**

Run: `cargo clippy -p dozerd --lib -- -D warnings && cargo fmt -- --check`
Expected: 全绿(`--all-targets` 会因为 `hook_events.rs` 还缺 `Client::list_conversations_with_summaries` 而编译失败,留到 Task 4 完成后一起跑)。

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/transcripts/mod.rs crates/dozerd/tests/hook_events.rs
git commit -m "feat(dozerd): ListConversationsWithSummaries handler, remove dead TurnGroup queries"
```

---

## Task 4: `dozer-client` 方法更新

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: Task 1/3 的协议类型与 handler。
- Produces: `Client::list_conversations_with_summaries(&self, cwd: &str, agent: Option<AgentKind>, limit: u32, offset: u32) -> Result<Vec<(ConversationSummary, Option<SessionSummaryPayload>)>>`。Task 6 的 `dozer-app` 直接调用。

- [ ] **Step 1: 实现新方法**

在 `crates/dozer-client/src/lib.rs` 的 `get_usage_summary` 方法之后追加:

```rust
    pub async fn list_conversations_with_summaries(
        &self,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<(ConversationSummary, Option<SessionSummaryPayload>)>> {
        match self
            .roundtrip(&Request::ListConversationsWithSummaries {
                cwd: cwd.into(),
                agent,
                limit,
                offset,
            })
            .await?
        {
            Reply::ConversationsWithSummaries { rows } => Ok(rows),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 2: 删除死方法**

删除 `Client::list_all_turn_groups`/`Client::list_session_turn_groups` 两个方法。
文件顶部 `use dozer_core::protocol::{...}` 导入列表里去掉 `TurnGroupEntry`/`TurnGroupSummary`
(若删完这两个方法后还有别处用到这两个类型,保留导入;用
`grep -n "TurnGroup" crates/dozer-client/src/lib.rs` 确认干净)。

- [ ] **Step 3: 更新集成测试**

在 `crates/dozer-client/tests/against_real_daemon.rs` 里删除引用
`list_all_turn_groups`/`list_session_turn_groups` 的测试(若存在——先
`grep -n "turn_group" crates/dozer-client/tests/against_real_daemon.rs` 确认)。

- [ ] **Step 4: 回头补全 Task 3 的联查集成测试**

回到 `crates/dozerd/tests/hook_events.rs` 的
`list_conversations_with_summaries_joins_correctly` 测试,现在
`Client::list_conversations_with_summaries` 已经存在。

Run: `cargo test -p dozerd --test hook_events list_conversations_with_summaries_joins_correctly`
Expected: PASS。

- [ ] **Step 5: 全量回归**

Run: `cargo build --workspace && cargo test -p dozerd -p dozer-core -p dozer-client && cargo clippy -p dozerd -p dozer-core -p dozer-client --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs crates/dozerd/tests/hook_events.rs
git commit -m "feat(dozer-client): add list_conversations_with_summaries, remove dead TurnGroup methods"
```

---

## Task 5: `dozer-app::conversation.rs` —— `SessionRow` 取代 `TurnGroupRow`

**Files:**
- Modify: `crates/dozer-app/src/conversation.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::{ConversationSummary, SessionSummaryPayload, SummaryStatus}`。
- Produces: `pub struct SessionRow { conversation_id, agent, last_ts, display_title, summary, summary_status }`、`SessionRow::from_row(&ConversationSummary, Option<&SessionSummaryPayload>) -> Self`。Task 6/7 使用这个类型。

- [ ] **Step 1: 写失败的测试**

替换 `crates/dozer-app/src/conversation.rs` 里 `turn_group_row_from_entry_maps_fields`
测试(以及它测的 `TurnGroupRow`/`from_entry`)为:

```rust
    #[test]
    fn session_row_from_row_uses_summary_title_and_text_when_present() {
        let c = ConversationSummary {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/c1.jsonl".into(),
            title: "原始标题".into(),
            first_ts: 1,
            last_ts: 100,
            turn_count: 3,
        };
        let s = SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: "总结标题".into(),
            summary: "总结全文".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 1,
        };
        let row = SessionRow::from_row(&c, Some(&s));
        assert_eq!(row.conversation_id, "c1");
        assert_eq!(row.agent, AgentKind::Claude);
        assert_eq!(row.last_ts, 100);
        assert_eq!(row.display_title, "总结标题");
        assert_eq!(row.summary.as_deref(), Some("总结全文"));
        assert_eq!(row.summary_status, Some(SummaryStatus::AiGenerated));
    }

    #[test]
    fn session_row_from_row_falls_back_to_conversation_title_without_summary() {
        let c = ConversationSummary {
            conversation_id: "c2".into(),
            agent: AgentKind::Codex,
            file_path: "/h/.codex/x/c2.jsonl".into(),
            title: "原始标题".into(),
            first_ts: 1,
            last_ts: 100,
            turn_count: 1,
        };
        let row = SessionRow::from_row(&c, None);
        assert_eq!(row.display_title, "原始标题");
        assert_eq!(row.summary, None);
        assert_eq!(row.summary_status, None);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app session_row`
Expected: 编译失败,`SessionRow` 未定义。

- [ ] **Step 3: 实现,删除 `TurnGroupRow`**

用以下内容替换 `conversation.rs` 里 `TurnGroupRow` 结构体及其 `impl` 块:

```rust
use dozer_core::protocol::{
    AgentKind, ConversationSummary, SessionSummaryPayload, SummaryStatus,
};

/// 会话列表一行(2026-08-27,取代按回合分组的 `TurnGroupRow`)。有总结用
/// 总结的标题/全文,没有降级用 `ConversationSummary.title`(旧数据/纯
/// shell/SSH/Codex/Kilo)。`summary` 存全文不截断——列表渲染时截断成
/// 预览,详情页直接整段展示,不为详情页单独发一次查询。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub last_ts: u64,
    pub display_title: String,
    pub summary: Option<String>,
    pub summary_status: Option<SummaryStatus>,
}

impl SessionRow {
    pub fn from_row(c: &ConversationSummary, s: Option<&SessionSummaryPayload>) -> Self {
        Self {
            conversation_id: c.conversation_id.clone(),
            agent: c.agent,
            last_ts: c.last_ts,
            display_title: s.map(|s| s.title.clone()).unwrap_or_else(|| c.title.clone()),
            summary: s.map(|s| s.summary.clone()),
            summary_status: s.map(|s| s.status),
        }
    }
}
```

原有 `TurnGroupEntry` 导入(`use dozer_core::protocol::{AgentKind, ConversationSummary, TurnGroupEntry};`)
删掉 `TurnGroupEntry`,换成上面这行(注意 `ConversationSummary` 本来就有导入,
`ConversationMeta`/`from_summary`/`is_current_conversation` 保持不变,不在本任务范围)。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app session_row`
Expected: 2 个测试全 PASS。

- [ ] **Step 5: clippy + fmt(这一步 dozer-app 全库还编译不过,因为 workspace.rs 还在用 `TurnGroupRow` ——只跑 conversation.rs 所在的语法检查,不要求整个 crate 通过)**

Run: `cargo check -p dozer-app --lib 2>&1 | grep "conversation.rs"`
Expected: `conversation.rs` 本身没有编译错误(其余文件因为还没改会报错,属于本任务之后的 Task 才会修,不在这里处理)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/conversation.rs
git commit -m "feat(dozer-app): add SessionRow, remove TurnGroupRow"
```

---

## Task 6: `dozer-app::workspace.rs` —— 状态字段与列表渲染

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `SessionRow`(Task 5)、`Client::list_conversations_with_summaries`(Task 4)。
- Produces: `Workspace.conversation_sessions: Option<Vec<SessionRow>>`(取代 `conversation_turn_groups`)、`filter_sessions`(取代 `filter_turn_groups`)、`Workspace::spawn_conversations_refresh`(取代 `spawn_all_turn_groups_refresh`)、`conversation_list_pane` 渲染改造。Task 8 的 `app.rs` 消费这几个改名后的符号。

- [ ] **Step 1: 改状态字段**

`Workspace` struct 里把:

```rust
    pub(crate) conversation_turn_groups: Option<Vec<TurnGroupRow>>,
```

改成:

```rust
    /// 当前项目全部 session 的列表(标题+总结),按最后活跃时间倒序;
    /// `None` = 还没加载过,`Some(空 vec)` = 加载完成但确实没有记录
    /// (2026-08-27,取代按回合分组拍平的 `conversation_turn_groups`)。
    pub(crate) conversation_sessions: Option<Vec<SessionRow>>,
```

`Workspace::from_restore`/构造函数里对应的初始化字段名同步改(搜
`conversation_turn_groups:` 找到所有初始化点)。

- [ ] **Step 2: 写失败的测试(改造 `filter_turn_groups` → `filter_sessions`)**

用以下内容替换 `filter_turn_groups` 函数的测试(若原先没有专门的单测,直接
新写;若已有,原地改造断言用的类型):

```rust
    fn session_row(title: &str, agent: AgentKind) -> SessionRow {
        SessionRow {
            conversation_id: title.to_string(),
            agent,
            last_ts: 0,
            display_title: title.to_string(),
            summary: None,
            summary_status: None,
        }
    }

    #[test]
    fn filter_sessions_matches_title_case_insensitively() {
        let rows = vec![
            session_row("改README", AgentKind::Claude),
            session_row("加安装说明", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "readme", None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "改README");
    }

    #[test]
    fn filter_sessions_by_agent() {
        let rows = vec![
            session_row("a", AgentKind::Claude),
            session_row("b", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "", Some(AgentKind::Codebuddy));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent, AgentKind::Codebuddy);
    }
```

- [ ] **Step 2b: 运行测试确认失败**

Run: `cargo test -p dozer-app filter_sessions`
Expected: 编译失败,`filter_sessions`/`SessionRow` 未在 workspace.rs 作用域内。

- [ ] **Step 3: 实现 `filter_sessions`,删除 `filter_turn_groups`**

用以下内容替换 `filter_turn_groups` 函数体(函数名、参数类型、返回类型均改):

```rust
/// 会话列表关键字 + agent 过滤:标题大小写不敏感子串匹配(空关键字不过滤
/// 标题这一维)叠加 agent 精确匹配(`None` = 不限)。拆成纯函数方便 headless
/// 单测(2026-08-27,取代 `filter_turn_groups`)。
fn filter_sessions<'a>(
    rows: &'a [SessionRow],
    query: &str,
    agent: Option<AgentKind>,
) -> Vec<&'a SessionRow> {
    let needle = query.to_lowercase();
    rows.iter()
        .filter(|r| agent.map(|a| r.agent == a).unwrap_or(true))
        .filter(|r| query.is_empty() || r.display_title.to_lowercase().contains(&needle))
        .collect()
}
```

- [ ] **Step 4: 改数据加载函数**

用以下内容替换 `spawn_all_turn_groups_refresh` 函数体(函数名、内部调用、
写回消息均改):

```rust
    /// 加载(或刷新)当前项目的会话列表(联查总结)。取代
    /// `spawn_all_turn_groups_refresh`(2026-08-27)。
    pub(crate) fn spawn_conversations_refresh(&self, io: &ShellIo) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let cwd = PathBuf::from(&project.path);
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let result = client
                .list_conversations_with_summaries(&cwd.to_string_lossy(), None, 500, 0)
                .await
                .map(|rows| {
                    rows.iter()
                        .map(|(c, s)| SessionRow::from_row(c, s.as_ref()))
                        .collect()
                })
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::ConversationSessionsRefreshed(project_id, result));
        });
    }
```

保留原函数的三处调用点(`from_restore`/`adopt_project`/`app.rs::delivery_checked`)
不在本任务改——它们调 `ws.spawn_all_turn_groups_refresh(io)`,函数改名后这三处
会编译报错,统一放到 Task 8(app.rs 那边一起改,避免同一处改动被拆成两个
半成品 commit)。**本任务允许 dozer-app 暂时编译不过**,Step 6 只跑局部检查,
不跑全量 build。

- [ ] **Step 5: 改列表渲染**

`conversation_list_pane` 函数体内:

- `let Some(rows) = ws.conversation_turn_groups.as_ref() else { ... }` 改成
  `let Some(rows) = ws.conversation_sessions.as_ref() else { ... }`。
- `let agents_present = conversation_agents_present(rows);` 若该函数签名是
  `&[TurnGroupRow]`,改成 `&[SessionRow]`(函数体内部逻辑不变,只是遍历
  `.agent` 字段,类型换了但字段名相同,函数体不需要改)。
- `let filtered = filter_turn_groups(rows, ...)` 改成
  `let filtered = filter_sessions(rows, &ws.conversation_search, ws.conversation_agent_filter);`。
- 循环 `for g in filtered.iter().take(visible)` 内部:
  - `g.title.clone()` 改成 `g.display_title.clone()`。
  - 副行文案:有 `g.summary` 就截断展示(截到 60 字符,超长加 `…`,复用
    `truncate_activity` 同款惯例——若 `workspace.rs` 里已有可复用的截断
    工具函数就直接用,没有就在本文件内写一个私有 `fn truncate_for_preview(s: &str) -> String`),
    没有 `summary` 就维持现状拼 `agent_label`+`relative_time_text`;当前
    session 的"● 当前"前缀逻辑不变。
  - `.on_press(Message::ConversationTurnGroupOpen(g.path.clone(), g.agent, g.start_turn_index, g.end_turn_index))`
    改成 `.on_press(Message::ConversationSessionOpen(g.conversation_id.clone(), g.agent))`
    (`Message::ConversationSessionOpen` 在 Task 8 才定义,本任务先把调用点
    改成这个新名字,允许暂时编译不过)。
- `is_current_conversation(&g.path, &opens)` 这一处需要重新考虑:
  `SessionRow` 没有 `path` 字段(只有 `conversation_id`)。`open_transcript_paths()`
  返回的是文件路径字符串列表——把判断逻辑改成"某个打开中的 transcript 路径
  的 `file_stem()` 等于 `g.conversation_id`",即在 `conversation.rs` 里新增
  一个 `pub fn is_current_conversation_id(conversation_id: &str, open_transcripts: &[String]) -> bool`
  (逻辑:`open_transcripts.iter().any(|p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str()) == Some(conversation_id))`),
  `conversation_list_pane` 里改调这个新函数。原有的 `is_current_conversation(path, ...)`
  连同它的单测一并删除(不再有任何调用点)。

- [ ] **Step 6: 局部编译检查**

Run: `cargo check -p dozer-app --lib 2>&1 | grep -c "error\[" `
Expected: 报错数量比 Task 5 结束时**减少**(部分错误已经被本任务修好,剩下的
是 `app.rs` 里 `ConversationTurnGroupOpen`/`ConversationTurnGroupsRefreshed`/
`spawn_all_turn_groups_refresh` 未定义,这些留给 Task 8)。不要求这一步
`cargo build` 通过。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/conversation.rs
git commit -m "feat(dozer-app): session-list state fields, filter, refresh, and rendering"
```

---

## Task 7: session 详情——`ReviewSource::Conversation` + 总结展示区 + 加载更多

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `SessionRow`(Task 5)。
- Produces: `ReviewSource::Conversation(String)`(取代死掉的 `FileRange`)、`ReviewView` 新增 `summary_title: Option<String>`/`summary_text: Option<String>`,删除 `prev_topic`/`next_topic`、`Workspace::spawn_review_load_conversation(&self, io: &ShellIo, conversation_id: String, after_turn_index: i64, limit: u32, append: bool)`(新的整段加载/追加逻辑,`append` 原样透传进 `Message::ReviewLoaded` 的第三个参数——`Message::ReviewLoaded` 本身的签名要扩到 4 元组才能接住这个参数,这一步在 Task 8 做,本任务先按 4 元组构造调用,允许暂时编译不过)。Task 8 的 `conversation_session_open`/`ConversationDetailLoadMore` 处理器使用这些。

- [ ] **Step 1: 改 `ReviewSource`/`ReviewView`,删 `TopicPreview`**

`pub enum ReviewSource` 从:

```rust
pub enum ReviewSource {
    Session(usize),
    FileRange(PathBuf, i64, i64),
}
```

改成:

```rust
pub enum ReviewSource {
    Session(usize),
    /// 对话面板的 session 详情:`conversation_id`,整段摊平加载,不再有
    /// "回合区间"概念(2026-08-27,取代 `FileRange`)。
    Conversation(String),
}
```

`pub struct ReviewView` 删除 `prev_topic`/`next_topic` 两个字段,新增:

```rust
    /// session 总结标题/全文(有则展示,无则详情页只显示回合列表)。
    /// 数据来自打开详情时已加载好的 `SessionRow`,不为此单独发请求。
    pub summary_title: Option<String>,
    pub summary_text: Option<String>,
```

删除 `pub struct TopicPreview` 和 `pub(crate) fn adjacent_topic_previews`
(及其单测:`adjacent_topic_previews_finds_same_session_neighbors_sorted_by_turn_index`/
`adjacent_topic_previews_none_at_session_boundaries`,用
`grep -n "adjacent_topic_previews\|TopicPreview" crates/dozer-app/src/workspace.rs`
定位全部引用后删除)。

其余构造 `ReviewView { ... }` 字面量的地方(测试 fixture,约 4114/4125/4139 行,
`grep -n "ReviewView {" crates/dozer-app/src/workspace.rs` 定位全部):
- 把 `source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),`
  改成 `source: ReviewSource::Conversation("a".into()),`(这几处测试原本只是
  拿 `FileRange` 当占位符,不测 `FileRange` 本身的行为,换成 `Conversation`
  语义等价)。
- 删掉 `prev_topic: None, next_topic: None,` 两行,加上
  `summary_title: None, summary_text: None,`。

`workspace.rs:4586` 附近 `review_should_refresh_on_turn` 的测试里
`&ReviewSource::FileRange(PathBuf::from("/t/x.jsonl"), 0, 4)` 同理改成
`&ReviewSource::Conversation("x".into())`。

- [ ] **Step 2: 写失败的测试(新加载函数)**

在 `workspace.rs` 测试模块里追加(验证"整段加载"和"追加式加载更多"两种
调用形态都产出正确的 `ReviewSource`/请求参数——用直接调纯逻辑部分的方式,
不起真实 daemon;若 `spawn_review_load` 目前没有可脱离 daemon 单测的纯函数
部分,这一步改成对 Step 3 实现的公开函数做 headless 编译检查,真机验证走
Task 9 的人工清单):

```rust
    #[test]
    fn review_source_conversation_matches_conversation_id_not_tab() {
        // Conversation 变体不该被 review_should_refresh_on_turn(只认
        // Session(tab_id))误判为需要跟随终端回合刷新。
        assert!(!review_should_refresh_on_turn(
            &ReviewSource::Conversation("c1".into()),
            3
        ));
    }
```

- [ ] **Step 3: 实现 `spawn_review_load_conversation`**

在 `spawn_review_load` 方法附近新增(不删除 `spawn_review_load`——它是通用的
"给一个 path+区间去查"函数,`ReviewSource::Session` 那条路径可能还在用类似
的加载逻辑;新方法专门服务 `Conversation` 这条路径,語意更直接,不用再算
`file_stem`):

```rust
    /// session 详情整段加载/"加载更多"追加(2026-08-27)。`append` 为
    /// `true` 时结果应追加进现有 `entries`(加载更多),`false` 时替换
    /// (首次打开)——原样透传进 `Message::ReviewLoaded` 的第三个参数,
    /// 实际的追加/替换逻辑在 `update()` 里处理,这里只负责发请求带上
    /// 这个标记。**`Message::ReviewLoaded` 的签名要在 Task 8 扩到 4 元组
    /// `(ProjectId, ReviewSource, bool, Result<...>)` 才能接住这里传的
    /// `append`——本任务落地后 `dozer-app` 暂时编译不过是预期状态**。
    pub(crate) fn spawn_review_load_conversation(
        &self,
        io: &ShellIo,
        conversation_id: String,
        after_turn_index: i64,
        limit: u32,
        append: bool,
    ) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        let source = ReviewSource::Conversation(conversation_id.clone());
        io.handle.spawn(async move {
            let result = client
                .get_conversation_turns(&conversation_id, after_turn_index, limit)
                .await
                .map(|turns| crate::transcript::review_entries_from_turns(&turns))
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::ReviewLoaded(project_id, source, append, result));
        });
    }
```

`CONVERSATION_DETAIL_PAGE_SIZE: u32 = 200`(首屏页大小,常量加在文件里
`CONVERSATION_PAGE_SIZE`(列表分页,20)附近,两者含义不同不要混用)。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo check -p dozer-app --lib 2>&1 | grep -c "error\["`
Expected: 报错数量继续减少(剩下的是 app.rs 里还没改的部分,留给 Task 8)。
`cargo test -p dozer-app review_source_conversation_matches_conversation_id_not_tab`
本身应该能单独跑通(这个测试不依赖 app.rs 那些还没改的符号)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): ReviewSource::Conversation, summary fields, whole-session loading"
```

---

## Task 8: `dozer-app::app.rs` —— Message 改造 + 调用点收口

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: Task 5-7 产出的全部符号(`SessionRow`/`filter_sessions`/`spawn_conversations_refresh`/`ReviewSource::Conversation`/`spawn_review_load_conversation`)。
- Produces: 无新公开接口——这是收口任务,让 `dozer-app` 重新编译通过。

- [ ] **Step 1: 改 `Message` 枚举**

把:

```rust
ConversationTurnGroupOpen(PathBuf, AgentKind, i64, i64),
```

改成:

```rust
ConversationSessionOpen(String, AgentKind),
/// "加载更多"追加当前 session 详情的下一页(`after_turn_index`)。
ConversationDetailLoadMore(String, i64),
```

`ConversationTurnGroupsRefreshed(ProjectId, Result<Vec<TurnGroupRow>, String>)`
改成 `ConversationSessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>)`。

- [ ] **Step 2: 改 `conversation_turn_group_open` → `conversation_session_open`**

用以下内容替换 `conversation_turn_group_open` 函数:

```rust
    fn conversation_session_open(&mut self, conversation_id: String, agent: AgentKind) {
        self.with_focused_project(move |ws, io| {
            let Some(row) = ws
                .conversation_sessions
                .as_ref()
                .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
            else {
                // 列表刷新与点击之间的竞态(极小概率):这一行已经不在当前
                // 列表里了,直接不打开详情,不 panic、不报错弹窗。
                return;
            };
            let source = ReviewSource::Conversation(conversation_id.clone());
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                agent,
                nonce: 0,
                summary_title: Some(row.display_title.clone()),
                summary_text: row.summary.clone(),
            });
            ws.spawn_review_load_conversation(
                io,
                conversation_id,
                -1,
                CONVERSATION_DETAIL_PAGE_SIZE,
                false,
            );
        });
    }
```

新增 `ConversationDetailLoadMore` 的处理(在 `update` 里 `Message::ConversationSessionOpen`
分支旁边):

```rust
            Message::ConversationSessionOpen(conversation_id, agent) => {
                self.conversation_session_open(conversation_id, agent);
            }
            Message::ConversationDetailLoadMore(conversation_id, after_turn_index) => {
                self.with_focused_project(|ws, io| {
                    ws.spawn_review_load_conversation(
                        io,
                        conversation_id,
                        after_turn_index,
                        CONVERSATION_DETAIL_PAGE_SIZE,
                        true,
                    );
                });
            }
```

- [ ] **Step 3: 改 `Message::ReviewLoaded` 签名与处理——追加 vs 替换**

`Message::ReviewLoaded` 的签名从 `(ProjectId, ReviewSource, Result<...>)` 改成
`(ProjectId, ReviewSource, bool, Result<...>)`(新增 `append` 参数,位置在
`ReviewSource` 之后、`Result` 之前——Task 7 的 `spawn_review_load_conversation`
已经按这个 4 元组构造消息,本步骤是补上枚举定义本身)。

`spawn_review_load`(`ReviewSource::Session` 那条路径,Task 7 未改动)的构造点
也要跟着补一个 `false`(它从不需要"加载更多"语义,恒替换)。

处理逻辑:现有"只要 `source == 已记录的 source` 就整段替换 `entries`"改成
按 `append` 参数分支——`append == true` 时 `rv.entries.extend(new_entries)`,
`append == false` 时整段替换(维持原逻辑)。

同时删除 `Message::ReviewLoaded` 处理逻辑里构造 `ReviewSnapshot` 时的
`prev_topic`/`next_topic` 两个字段(`ReviewSnapshot` struct 本身也删这两个
字段,搜 `struct ReviewSnapshot` 定位)。

- [ ] **Step 4: 收口三处 `spawn_all_turn_groups_refresh` 调用点**

`Workspace::from_restore`(约 619 行)、`Workspace::adopt_project`(约 1284 行)、
`App::delivery_checked`(约 6550 行)三处 `ws.spawn_all_turn_groups_refresh(io);`
改成 `ws.spawn_conversations_refresh(io);`。

`Message::ConversationTurnGroupsRefreshed(project_id, result)` 处理分支改成
`Message::ConversationSessionsRefreshed`,内部 `ws.conversation_turn_groups = Some(rows)`
改成 `ws.conversation_sessions = Some(rows)`。

- [ ] **Step 5: 全量编译**

Run: `cargo build --workspace 2>&1 | grep "error\[" | sort -u`
Expected: 空输出(无编译错误)。若还有残留错误,逐条定位——大概率是某个
测试 fixture 或次要调用点(比如 `PanelKind::Conversations` 打开时是否有
额外触发逻辑)漏改,不在本计划穷举范围内的按实际报错信息修,遵循"改名
后旧符号处处收口"这个原则。

- [ ] **Step 6: 全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): wire session-list navigation into app.rs Message/update"
```

---

## Task 9: 全量验证 + 真机验证清单

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建/测试/静态检查**

Run: `cargo build --workspace`
Expected: 成功。

Run: `cargo test -p dozerd -p dozer-app -p dozer-core -p dozer-client -p dozer-mcp`
Expected: 全绿。

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 无警告(pre-existing 的 `todo.rs::ContentSubmit` dead_code 警告若仍
存在,先确认是否本计划改动引入——若与本计划任何一个 commit 无关,记录下来
告知用户,不在本计划范围内顺手修)。

Run: `cargo fmt -- --check`
Expected: 无需改动。

- [ ] **Step 2: 真机验证(单测覆盖不到,必须人工在真实 GUI 里跑)**

逐条对照 spec `docs/superpowers/specs/2026-08-27-conversation-panel-session-navigation-design.md`
"测试策略"第 6 条清单手动验证:

1. 有总结的 session:列表行显示总结标题+预览,点开详情看到标题+摘要全文+
   完整回合列表。
2. 没有总结的历史 session(旧数据/纯 shell):列表行降级显示原标题,点开
   详情只有回合列表、没有总结展示区。
3. 长会话(回合数超过 `CONVERSATION_DETAIL_PAGE_SIZE`):详情页"加载更多"
   正确追加而不是替换已加载内容。
4. 搜索/agent 过滤:输入关键字/切换 agent 过滤器,列表正确收窄。
5. 镜像态(`app.panel_mirrored(PanelKind::Conversations)`):左右互换后
   列表点击→详情联动依然正确。
6. 当前活跃 session(仍在某个终端 tab 里打开)在列表里应正确标"● 当前"
   (验证 `is_current_conversation_id` 替换旧 `is_current_conversation` 后
   行为等价)。

- [ ] **Step 3: 记录验证结果**

若真机验证全部通过,在 PR 描述或提交信息里注明"真机验证 1-6 项已过";若
某项失败,回退到对应 Task 定位问题,不要在真机验证失败的情况下声称任务
完成。
