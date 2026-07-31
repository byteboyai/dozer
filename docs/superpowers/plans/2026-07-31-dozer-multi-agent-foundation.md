# Dozer 多 Agent 基础协议改动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 dozer 的协议/dozerd/dozer-app 全链路加上 `AgentKind`（Unknown/Claude/Codebuddy/Opencode）维度，使"这个会话是哪个 agent 在跑"能从 hook 事件一路传到 GUI，且现有 Claude 行为在改造后逐字节保持不变。

**Architecture:** 在 `dozer-core::protocol` 新增 `AgentKind` 枚举，贯穿 `SessionInfo`/`Request::HookEvent`/`Reply::AgentEvent` 三个消息类型；`dozerd::Session` 存一份 `agent` 状态、随 hook 事件坐实并广播；`dozer-hook` 的调用形态从 `dozer-hook <event>` 改成 `dozer-hook <agent> <event>`（本计划只接 Claude 分支）；`dozer-app` 的 transcript 解析器与对话历史扫描器都改成按 `AgentKind` 分派（Codebuddy 分支本计划先返回空结果/空目录，行为在后续 CodeBuddy 适配计划里补齐——这不是占位符，是"未实现的 agent 一律不产出数据"这一条真实、可测试的降级路径）。

**Tech Stack:** Rust 2024，workspace crates：`dozer-core`（协议）、`dozerd`（daemon）、`dozer-client`（GUI 侧 client）、`dozer-hook`（hook 转发二进制）、`dozer-app`（iced GUI）。

## Global Constraints

- 本计划交付后，Claude Code 相关的既有行为（hook 安装、事件转发、四态机、transcript 解析、对话历史扫描）必须逐字节保持不变——唯一允许的观测差异是 `SessionInfo.agent`/`Reply::AgentEvent.agent`/`ConversationMeta.agent` 现在会正确报告 `AgentKind::Claude` 而不是无该字段/写死字符串。
- 协议新增字段一律 `#[serde(default)]`，缺字段的老协议帧必须回落到 `AgentKind::Unknown`（沿用 `agent_state`/`transcript_path`/`project_id` 已验证过的兼容模式）。
- `dozerd::agent_state_for(event: &str) -> Option<AgentState>` 本计划不改签名、不改实现——规范事件名翻译是每个 agent 适配层自己的职责（见 spec §3），不属于本计划范围。
- 不加"agent 选择器"或任何会话启动侧 UI 改动（已与用户核对，见 spec §2）。
- 参照 spec：`docs/superpowers/specs/2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md`。

---

## Task 1: `AgentKind` 类型 + `label()` 辅助方法

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:1-12`（枚举定义插入到 `AgentState` 之后）

**Interfaces:**
- Produces: `pub enum AgentKind { Unknown, Claude, Codebuddy, Opencode }`（`Default` = `Unknown`，`Serialize`/`Deserialize` 用 `snake_case`）；`impl AgentKind { pub fn label(&self) -> &'static str }`。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozer-core/src/protocol.rs` 的 `#[cfg(test)] mod tests` 块内新增：

```rust
    #[test]
    fn agent_kind_defaults_to_unknown() {
        assert_eq!(AgentKind::default(), AgentKind::Unknown);
    }

    #[test]
    fn agent_kind_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&AgentKind::Codebuddy).unwrap(), "\"codebuddy\"");
        assert_eq!(serde_json::to_string(&AgentKind::Opencode).unwrap(), "\"opencode\"");
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"claude\"").unwrap(),
            AgentKind::Claude
        );
    }

    #[test]
    fn agent_kind_label_matches_variant() {
        assert_eq!(AgentKind::Unknown.label(), "未知");
        assert_eq!(AgentKind::Claude.label(), "claude");
        assert_eq!(AgentKind::Codebuddy.label(), "codebuddy");
        assert_eq!(AgentKind::Opencode.label(), "opencode");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core agent_kind -- --nocapture`
Expected: FAIL，报 `AgentKind` 未定义。

- [ ] **Step 3: 实现**

在 `crates/dozer-core/src/protocol.rs` 第 12 行（`AgentState` 定义结束）之后插入：

```rust
/// 会话当前归属的 agent（协议层从 P1e-P1j 时代的"隐式恒 Claude"升级为
/// 显式字段；P2b 多 agent 支持第一步）。`Unknown` 是首个 hook 事件到达前
/// 的默认值，也是老协议帧缺该字段时的回落值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Unknown,
    Claude,
    Codebuddy,
    Opencode,
}

impl AgentKind {
    /// 展示用短标签（对话历史副行、GUI 角标）。
    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Unknown => "未知",
            AgentKind::Claude => "claude",
            AgentKind::Codebuddy => "codebuddy",
            AgentKind::Opencode => "opencode",
        }
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core agent_kind -- --nocapture`
Expected: PASS（3 个新测试）

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add AgentKind enum with snake_case serde and label()"
```

---

## Task 2: `SessionInfo.agent` 字段

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:23-42`（`SessionInfo` 结构体）
- Modify: `crates/dozer-core/src/protocol.rs`（`session_info_carries_project_id`、`old_session_info_without_project_id_decodes_none` 等既有测试所在的 `mod tests` 块，新增用例）

**Interfaces:**
- Consumes: `AgentKind`（Task 1）。
- Produces: `SessionInfo.agent: AgentKind`（`#[serde(default)]`）。

- [ ] **Step 1: 写失败的测试**

```rust
    #[test]
    fn session_info_carries_agent() {
        let info = SessionInfo {
            id: "a".into(),
            name: "n".into(),
            command: "/bin/sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Claude,
        };
        let line = encode_line(&info);
        let back: SessionInfo = decode_line(line.trim()).unwrap();
        assert_eq!(back.agent, AgentKind::Claude);
    }

    #[test]
    fn old_session_info_without_agent_decodes_unknown() {
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.agent, AgentKind::Unknown);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core session_info_carries_agent old_session_info_without_agent -- --nocapture`
Expected: FAIL，`SessionInfo` 没有 `agent` 字段，编译错误。

- [ ] **Step 3: 实现**

`crates/dozer-core/src/protocol.rs` 的 `SessionInfo` 结构体（现有 `project_id` 字段之后）加：

```rust
    /// 会话归属的 agent；首个 hook 事件到达前恒 `Unknown`（P2b）。
    #[serde(default)]
    pub agent: AgentKind,
```

同时修复既有的三处 Rust 侧 `SessionInfo { .. }` 字面量构造（编译器会报缺字段），本任务只需要在 `protocol.rs` 自己的测试模块里补——`session_info_carries_project_id` 测试（约第 320 行）加一行 `agent: AgentKind::Unknown,`。`dozerd/src/session.rs` 的 `info()` 构造点留给 Task 5 处理（那里要填真实值，不属于本任务）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core -- --nocapture`
Expected: `dozer-core` 包内测试全绿（`dozerd`/`dozer-app` 此刻应该编译失败，属预期，Task 5+ 会修复）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add agent field to SessionInfo"
```

---

## Task 3: `Request::HookEvent.agent` 字段

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:75-81`（`Request::HookEvent` 变体）
- Modify: `crates/dozer-core/src/protocol.rs`（`hook_event_roundtrips` 测试）

**Interfaces:**
- Consumes: `AgentKind`（Task 1）。
- Produces: `Request::HookEvent { session_id: String, agent: AgentKind, event: String, ts_ms: u64, data: serde_json::Value }`。

- [ ] **Step 1: 写失败的测试**

替换现有 `hook_event_roundtrips` 测试体（原测试构造缺 `agent` 字段，直接改到位）：

```rust
    #[test]
    fn hook_event_roundtrips() {
        let req = Request::HookEvent {
            session_id: "s1".into(),
            agent: AgentKind::Claude,
            event: "Stop".into(),
            ts_ms: 123,
            data: serde_json::json!({"transcript_path": "/tmp/t.jsonl"}),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn old_hook_event_without_agent_decodes_unknown() {
        let old = r#"{"type":"hook_event","session_id":"s1","event":"Stop","ts_ms":1,"data":null}"#;
        match decode_line::<Request>(old).unwrap() {
            Request::HookEvent { agent, .. } => assert_eq!(agent, AgentKind::Unknown),
            other => panic!("{other:?}"),
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core hook_event -- --nocapture`
Expected: FAIL，编译错误（`HookEvent` 缺 `agent` 字段 / 字面量字段不匹配）。

- [ ] **Step 3: 实现**

`Request::HookEvent` 变体（现有 `session_id` 之后）加：

```rust
        /// 触发这次事件的 agent；由 dozer-hook/opencode 插件在装的时候
        /// 写死，不是猜出来的（spec §3）。
        agent: AgentKind,
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core hook_event -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add agent field to Request::HookEvent"
```

---

## Task 4: `Reply::AgentEvent.agent` 字段

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:133-141`（`Reply::AgentEvent` 变体）
- Modify: `crates/dozer-core/src/protocol.rs`（`agent_event_reply_tags_snake_case`、`agent_event_carries_transcript_path` 测试）

**Interfaces:**
- Consumes: `AgentKind`（Task 1）。
- Produces: `Reply::AgentEvent { session_id: String, agent: AgentKind, state: AgentState, event: String, ts_ms: u64, transcript_path: Option<String> }`。

- [ ] **Step 1: 写失败的测试**

更新两个既有测试（就地改，别新建重复用例）：

```rust
    #[test]
    fn agent_event_reply_tags_snake_case() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s1".into(),
            agent: AgentKind::Claude,
            state: AgentState::AwaitingInput,
            event: "Notification".into(),
            ts_ms: 5,
            transcript_path: None,
        });
        assert!(line.contains(r#""type":"agent_event""#));
        assert!(line.contains(r#""state":"awaiting_input""#));
        assert!(line.contains(r#""agent":"claude""#));
    }
```

```rust
    #[test]
    fn agent_event_carries_transcript_path() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s".into(),
            agent: AgentKind::Codebuddy,
            state: AgentState::Running,
            event: "UserPromptSubmit".into(),
            ts_ms: 1,
            transcript_path: Some("/t/x.jsonl".into()),
        });
        assert!(line.contains("/t/x.jsonl"));
        match decode_line::<Reply>(line.trim()).unwrap() {
            Reply::AgentEvent {
                agent,
                transcript_path,
                ..
            } => {
                assert_eq!(agent, AgentKind::Codebuddy);
                assert_eq!(transcript_path.as_deref(), Some("/t/x.jsonl"));
            }
            other => panic!("{other:?}"),
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core agent_event -- --nocapture`
Expected: FAIL，编译错误。

- [ ] **Step 3: 实现**

`Reply::AgentEvent` 变体（现有 `session_id` 之后）加：

```rust
        agent: AgentKind,
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core -- --nocapture`
Expected: `dozer-core` 全绿。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add agent field to Reply::AgentEvent"
```

---

## Task 5: `dozerd::Session` 存储与广播 `agent`

**Files:**
- Modify: `crates/dozerd/src/session.rs:10-26`（`SessionEvent::Agent` 变体）
- Modify: `crates/dozerd/src/session.rs:39-51`（`Session` 结构体）
- Modify: `crates/dozerd/src/session.rs:138-182`（`spawn`/`info`/`set_agent_state`，新增 `set_agent`）
- Modify: `crates/dozerd/src/session.rs`（`#[cfg(test)] mod tests`，新增/更新用例）

**Interfaces:**
- Consumes: `AgentKind`（Task 1，通过 `dozer_core::protocol::AgentKind` 引入）。
- Produces: `Session::set_agent(&self, agent: AgentKind)`；`Session::info()` 现在带正确 `agent`；`SessionEvent::Agent { agent: AgentKind, state, event, ts_ms, transcript_path }`。

- [ ] **Step 1: 写失败的测试**

在现有 `transcript_path_stored_and_in_info_and_broadcast` 测试之后新增（`Session::spawn` 之后、任何 `set_agent` 调用之前，agent 必须是默认值 `Unknown`）：

```rust
    #[tokio::test]
    async fn set_agent_updates_info_and_is_included_in_broadcast() {
        let s = Session::spawn(spec("sleep 5")).unwrap();
        assert_eq!(s.info().agent, AgentKind::Unknown);
        let mut rx = s.subscribe();
        s.set_agent(AgentKind::Codebuddy);
        assert_eq!(s.info().agent, AgentKind::Codebuddy);
        s.set_agent_state(AgentState::Running, "PreToolUse", 1);
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                SessionEvent::Agent { agent, .. } => {
                    assert_eq!(agent, AgentKind::Codebuddy);
                    break;
                }
                _ => continue,
            }
        }
        let _ = s.kill();
    }
```

同时给顶部 `use` 加 `AgentKind`：`use dozer_core::protocol::{AgentKind, AgentState, SessionInfo};`（第 3 行）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd set_agent_updates_info -- --nocapture`
Expected: FAIL，`Session` 无 `set_agent` 方法 / `SessionEvent::Agent` 无 `agent` 字段，编译错误。

- [ ] **Step 3: 实现**

`SessionEvent::Agent` 变体（第 19-25 行）加字段：

```rust
    /// hook 事件驱动的 agent 状态变更（P1e）。
    Agent {
        agent: AgentKind,
        state: AgentState,
        event: String,
        ts_ms: u64,
        transcript_path: Option<String>,
    },
```

`Session` 结构体（第 39-51 行）加字段（紧跟 `agent_state`）：

```rust
    agent: Mutex<AgentKind>,
```

`spawn()` 里的构造字面量（第 138-150 行）加：

```rust
            agent: Mutex::new(AgentKind::default()),
```

`info()`（第 153-165 行）加：

```rust
            agent: *self.agent.lock().expect("agent lock"),
```

新增方法（紧跟 `set_transcript_path`，第 170 行之后）：

```rust
    /// 记下会话归属的 agent（首个 hook 事件到达时坐实；覆盖旧值）。
    pub fn set_agent(&self, agent: AgentKind) {
        *self.agent.lock().expect("agent lock") = agent;
    }
```

`set_agent_state`（第 172-182 行）广播时带上当前 agent：

```rust
    pub fn set_agent_state(&self, state: AgentState, event: &str, ts_ms: u64) {
        *self.agent_state.lock().expect("agent_state lock") = state;
        let transcript_path = self.transcript_path.lock().expect("tp lock").clone();
        let agent = *self.agent.lock().expect("agent lock");
        let _ = self.tx.send(SessionEvent::Agent {
            agent,
            state,
            event: event.to_string(),
            ts_ms,
            transcript_path,
        });
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd -- --nocapture`
Expected: `dozerd` 库内测试全绿（`server.rs`/`registry.rs` 等消费方此刻应该编译失败，属预期，Task 6 会修复）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/session.rs
git commit -m "feat(dozerd): Session tracks and broadcasts AgentKind"
```

---

## Task 6: `dozerd::server` 接线 + 集成测试

**Files:**
- Modify: `crates/dozerd/src/server.rs:129-147`（`Request::HookEvent` 处理分支）
- Modify: `crates/dozerd/src/server.rs:222-225`（`SessionEvent::Agent` → `Reply::AgentEvent` 转发）
- Modify: `crates/dozerd/tests/hook_events.rs:78-89,113-126`（两处 `Request::HookEvent` 字面量补字段）
- Modify: `crates/dozerd/tests/hook_events.rs`（`hook_event_reaches_attached_client_and_list` 测试断言新增 `agent` 检查）

**Interfaces:**
- Consumes: Task 3/4/5 产出（`Request::HookEvent.agent`、`Reply::AgentEvent.agent`、`Session::set_agent`）。
- Produces: 端到端链路——HookEvent 带 agent 进来 → `Session::set_agent` 坐实 → `ListSessions`/`AgentEvent` 都能看到正确 agent。

- [ ] **Step 1: 写失败的测试**

修改 `crates/dozerd/tests/hook_events.rs` 第 76-89 行那次 `send_req` 调用，加 `agent`：

```rust
    match send_req(
        &sock,
        &Request::HookEvent {
            session_id: created.id.clone(),
            agent: dozer_core::protocol::AgentKind::Codebuddy,
            event: "Stop".into(),
            ts_ms: 7,
            data: serde_json::Value::Null,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
```

紧接着"A 在 PTY 噪音里等到 AgentEvent"那段循环（原第 92-103 行），断言里加 agent 检查：

```rust
    loop {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("AgentEvent within 5s")
            .expect("io ok")
            .expect("stream open");
        if let Ok(Reply::AgentEvent { agent, state, event, .. }) = decode_line::<Reply>(&line) {
            assert_eq!(agent, dozer_core::protocol::AgentKind::Codebuddy);
            assert_eq!(state, AgentState::TurnEnded);
            assert_eq!(event, "Stop");
            break;
        }
    }
```

`ListSessions` 断言（原第 105-110 行）加一行：

```rust
    match send_req(&sock, &Request::ListSessions).await {
        Reply::Sessions { sessions } => {
            assert_eq!(sessions[0].agent_state, AgentState::TurnEnded);
            assert_eq!(sessions[0].agent, dozer_core::protocol::AgentKind::Codebuddy);
        }
        other => panic!("{other:?}"),
    }
```

第 113-126 行"未知会话"那次 `Request::HookEvent` 字面量也要补字段（否则编译不过），`agent` 随便给个确定值即可：

```rust
    match send_req(
        &sock,
        &Request::HookEvent {
            session_id: "ghost".into(),
            agent: dozer_core::protocol::AgentKind::Claude,
            event: "Stop".into(),
            ts_ms: 8,
            data: serde_json::Value::Null,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd --test hook_events -- --nocapture`
Expected: FAIL，编译错误（`server.rs` 里 `Request::HookEvent { session_id, event, ts_ms, data }` 解构缺 `agent` 分支、`SessionEvent::Agent` 解构缺字段）。

- [ ] **Step 3: 实现**

`server.rs` 第 129-147 行的 `Request::HookEvent` 分支：

```rust
                        Request::HookEvent { session_id, agent, event, ts_ms, data } => {
                            match registry.get(&session_id) {
                                None => {
                                    tracing::debug!(%session_id, %event, "hook 事件的会话不存在，丢弃");
                                }
                                Some(s) => {
                                    s.set_agent(agent);
                                    if let Some(tp) =
                                        data.get("transcript_path").and_then(|v| v.as_str())
                                    {
                                        s.set_transcript_path(tp);
                                    }
                                    match agent_state_for(&event) {
                                        Some(state) => s.set_agent_state(state, &event, ts_ms),
                                        None => tracing::debug!(%event, "未知 hook 事件，不改状态"),
                                    }
                                }
                            }
                            Reply::Ok
                        }
```

第 222-225 行的转发：

```rust
                    Ok(SessionEvent::Agent { agent, state, event, ts_ms, transcript_path }) => {
                        let reply = Reply::AgentEvent { session_id: sid, agent, state, event, ts_ms, transcript_path };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd -- --nocapture`
Expected: `dozerd` 库测试 + `hook_events.rs`/`session_survival.rs` 集成测试全绿。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/tests/hook_events.rs
git commit -m "feat(dozerd): wire agent through HookEvent -> Session -> AgentEvent"
```

---

## Task 7: `dozer-hook` CLI 形态改为 `<agent> <event>`

**Files:**
- Modify: `crates/dozer-hook/src/main.rs`（整份改写 `forward`，`main` 的分派逻辑跟着调整）
- Modify: `crates/dozer-hook/src/install.rs:71-82`（写入 hook command 的格式串）
- Modify: `crates/dozer-hook/src/install.rs`（`install_creates_settings_and_registers_all_events` 测试断言新增"含 claude"检查）

**Interfaces:**
- Consumes: `AgentKind`（Task 1）。
- Produces: 安装后 `~/.claude/settings.json` 里的 hook command 形如 `<exe> claude <event>`；`dozer-hook claude <event>` 转发时 `Request::HookEvent.agent = AgentKind::Claude`。

- [ ] **Step 1: 写失败的测试**

`crates/dozer-hook/src/install.rs` 的 `install_creates_settings_and_registers_all_events` 测试（现有 `cmd.ends_with(ev)` 断言那行）之后加一行：

```rust
                cmd.contains(" claude "),
                "{cmd}"
```

完整替换该测试里的断言块（原第 130-134 行）为：

```rust
            assert!(
                cmd.contains("dozer-hook") || cmd.contains("dozer_hook"),
                "{cmd}"
            );
            assert!(cmd.contains(" claude "), "{cmd}: 应携带 agent 标识");
            assert!(cmd.ends_with(ev), "{cmd}");
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook install_creates_settings -- --nocapture`
Expected: FAIL（当前写入的是 `{exe} {ev}`，不含 `" claude "`）。

- [ ] **Step 3: 实现**

`install.rs` 第 77-81 行（`run_at` 里 `if install` 分支）：

```rust
        if install {
            arr.push(json!({
                "hooks": [{ "type": "command", "command": format!("{exe} claude {ev}") }]
            }));
        }
```

`main.rs` 整份改写：

```rust
mod install;

use dozer_core::protocol::{AgentKind, Request, encode_line};
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arg1 = std::env::args().nth(1);
    match arg1.as_deref() {
        Some("install") => std::process::exit(install::run_at(&install::settings_path(), true)),
        Some("uninstall") => std::process::exit(install::run_at(&install::settings_path(), false)),
        Some(agent_arg) => {
            let agent = parse_agent(agent_arg);
            let event_arg = std::env::args().nth(2);
            forward(agent, event_arg.as_deref());
        }
        None => forward(AgentKind::Unknown, None),
    }
}

/// CLI 里的 agent 名字（安装时写死进 hook command）→ `AgentKind`。
/// 未识别的字符串不 panic，落回 `Unknown`——保持 P1e 定下的"恒静默、
/// 绝不因为解析失败拖慢 agent"这条错误处理哲学。
fn parse_agent(arg: &str) -> AgentKind {
    match arg {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        _ => AgentKind::Unknown,
    }
}

/// 把一次 hook 调用转发给 dozerd。恒静默、恒成功退出——绝不拖慢 agent
/// （spec P1e 错误处理：dozerd 不在/超时/畸形输入一律吞掉）。
fn forward(agent: AgentKind, event_arg: Option<&str>) {
    // 非 Dozer 会话（无注入的会话 id）：与 dozerd 无关，直接退出
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        return;
    };
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let data = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let event = match event_arg {
        Some(e) => e.to_string(),
        None => data
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let req = Request::HookEvent {
        session_id,
        agent,
        event,
        ts_ms,
        data,
    };

    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(dozer_core::paths::socket_path())
    else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.write_all(encode_line(&req).as_bytes());
    // 单向语义：不读应答，发完即走
}
```

> 注：原 `main.rs` 没有单元测试（`forward`/`main` 直接读环境变量和 stdin，端到端行为已经由 Task 6 的 `dozerd/tests/hook_events.rs` 间接覆盖 dozerd 那一侧；`parse_agent` 是本任务新增的纯函数，下面补测试）。

在 `main.rs` 底部新增：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_recognizes_known_names() {
        assert_eq!(parse_agent("claude"), AgentKind::Claude);
        assert_eq!(parse_agent("codebuddy"), AgentKind::Codebuddy);
        assert_eq!(parse_agent("opencode"), AgentKind::Opencode);
    }

    #[test]
    fn parse_agent_falls_back_to_unknown() {
        assert_eq!(parse_agent("something-else"), AgentKind::Unknown);
        assert_eq!(parse_agent(""), AgentKind::Unknown);
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS（`install.rs` 4 个既有测试 + 新断言、`main.rs` 2 个新测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/main.rs crates/dozer-hook/src/install.rs
git commit -m "feat(dozer-hook): CLI shape becomes <agent> <event>, Claude installer writes agent-tagged command"
```

---

## Task 8: `dozer-client::TermEvent::Agent` 带 `agent`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs:10-23`（`TermEvent` 枚举）
- Modify: `crates/dozer-client/src/lib.rs:232-234`（`Reply::AgentEvent` → `TermEvent::Agent` 转换）

**Interfaces:**
- Consumes: `Reply::AgentEvent.agent`（Task 4）。
- Produces: `TermEvent::Agent { agent: AgentKind, state: AgentState, transcript_path: Option<String> }`。

- [ ] **Step 1: 写失败的测试**

`dozer-client` 目前没有针对 `TermEvent` 转换逻辑的单元测试（这段代码嵌在 `tokio::spawn` 的读循环闭包里，走的是 `crates/dozer-client/tests/against_real_daemon.rs` 端到端覆盖）。本任务不新增测试文件，改成先跑一次确认当前测试基线通过，再改代码，再跑一次确认没有回归——这是"类型穿线、无新行为"改动的合理验证方式（跟 spec §9 里"OpenCode 分支因复用 Claude 解析代码，靠现有测试覆盖，不新写"是同一类判断）。

Run: `cargo test -p dozer-client -- --nocapture`
Expected: 当前应该编译失败（Task 4 已经改了 `Reply::AgentEvent` 的字段集合，`lib.rs:232` 那行 `Ok(Reply::AgentEvent { state, transcript_path, .. })` 本身仍能编译——`..` 会吞掉新增的 `agent` 字段——但 `TermEvent::Agent` 结构体这一步还没加字段，属于本任务要做的事，先确认基线红/绿状态即可）。

- [ ] **Step 2: 确认当前基线状态**

Run: `cargo build -p dozer-client`
Expected: 在 Task 1-6 落地后应该能编译通过（`..` 模式兼容新字段），这一步只是留证据："改之前是绿的"。

- [ ] **Step 3: 实现**

`TermEvent` 枚举（第 10-23 行）：

```rust
#[derive(Debug)]
pub enum TermEvent {
    Output(Vec<u8>),
    Exited(Option<i32>),
    Lagged,
    Disconnected,
    /// 本会话 agent 状态变更（hook 事件驱动，dozerd 广播）。
    Agent {
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    },
}
```

顶部 import（第 4-6 行）加 `AgentKind`：

```rust
use dozer_core::protocol::{
    AgentKind, AgentState, ProjectInfo, Reply, Request, SessionInfo, decode_line, encode_line,
};
```

第 232-234 行：

```rust
                                    Ok(Reply::AgentEvent { agent, state, transcript_path, .. }) => {
                                        TermEvent::Agent { agent, state, transcript_path }
                                    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-client -- --nocapture`
Expected: PASS，无回归。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): thread agent through TermEvent::Agent"
```

---

## Task 9: `dozer-app::conversation` 按 agent 分派 + 多目录合并

**Files:**
- Modify: `crates/dozer-app/src/conversation.rs`（整份文件：`ConversationMeta.agent` 类型、目录解析函数、`list_conversations` 签名、新增 `list_all_conversations`）

**Interfaces:**
- Consumes: `AgentKind`（Task 1）。
- Produces:
  - `pub struct ConversationMeta { path, title, modified_ms, size_bytes, agent: AgentKind }`
  - `pub fn claude_project_dir(cwd: &Path) -> PathBuf`（不变）
  - `pub fn codebuddy_project_dir(cwd: &Path) -> PathBuf`（新增）
  - `pub fn opencode_project_dir(cwd: &Path) -> PathBuf`（新增）
  - `pub fn list_conversations(agent: AgentKind, dir: &Path) -> Vec<ConversationMeta>`（签名新增 `agent` 参数）
  - `pub fn list_all_conversations(cwd: &Path) -> Vec<ConversationMeta>`（新增：合并三个 agent 目录、按 mtime 倒序）

- [ ] **Step 1: 写失败的测试**

替换 `conversation.rs` 现有测试模块里的这几个用例（其余 `conversation_title`/`is_current_conversation` 测试不受影响，原样保留）：

```rust
    #[test]
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn codebuddy_dir_uses_codebuddy_root() {
        let d = codebuddy_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.codebuddy/projects/-a-b-c"));
    }

    #[test]
    fn opencode_dir_lives_under_dozer_data_dir() {
        let d = opencode_project_dir(std::path::Path::new("/a/b/c"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.dozer/agents/opencode/projects/-a-b-c")
        );
    }

    #[test]
    fn list_sorts_by_mtime_desc_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("one.jsonl");
        let p2 = dir.path().join("two.jsonl");
        std::fs::write(
            &p1,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第一个\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            &p2,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二个\"}}\n",
        )
        .unwrap();
        let list = list_conversations(AgentKind::Claude, dir.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "第二个", "mtime 倒序:后写的在前");
        assert_eq!(list[0].agent, AgentKind::Claude);
        assert!(list[0].size_bytes > 0);
        assert!(
            list_conversations(AgentKind::Claude, std::path::Path::new("/no/such/dir")).is_empty()
        );
    }

    #[test]
    fn list_conversations_tags_requested_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();
        let list = list_conversations(AgentKind::Codebuddy, dir.path());
        assert_eq!(list[0].agent, AgentKind::Codebuddy);
    }

    #[test]
    fn list_all_conversations_merges_three_dirs_sorted_by_mtime() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: 测试串行执行，临时改 HOME 后立即恢复；claude/codebuddy/opencode
        // 三个目录解析函数都读 HOME 环境变量。
        let prev_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", home.path()) };

        let cwd = std::path::Path::new("/proj");
        let claude_dir = claude_project_dir(cwd);
        let codebuddy_dir = codebuddy_project_dir(cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codebuddy_dir).unwrap();
        std::fs::write(
            claude_dir.join("a.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"claude 对话\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            codebuddy_dir.join("b.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"codebuddy 对话\"}}\n",
        )
        .unwrap();

        let list = list_all_conversations(cwd);

        match prev_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].agent, AgentKind::Codebuddy, "后写入的在前");
        assert_eq!(list[1].agent, AgentKind::Claude);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app conversation:: -- --nocapture`
Expected: FAIL（`codebuddy_project_dir`/`opencode_project_dir`/`list_all_conversations` 不存在；`list_conversations` 签名不匹配；`ConversationMeta.agent` 类型不匹配）。

- [ ] **Step 3: 实现**

整份改写 `crates/dozer-app/src/conversation.rs`：

```rust
//! 多 agent 对话来源（P1j 起步，P2b 扩展到 CodeBuddy/OpenCode）：扫描每个
//! agent 各自的落盘目录，列出该项目的历史对话（JSONL），取首句人类发言
//! 当标题。纯 IO + 纯解析；dozerd 不参与。

use dozer_core::protocol::AgentKind;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMeta {
    pub path: PathBuf,
    pub title: String,
    pub modified_ms: u64,
    pub size_bytes: u64,
    pub agent: AgentKind,
}

fn project_key(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// cwd → Claude 存储目录：`~/.claude/projects/<cwd 中 '/' 换 '-'>`。
pub fn claude_project_dir(cwd: &Path) -> PathBuf {
    home_dir().join(".claude").join("projects").join(project_key(cwd))
}

/// cwd → CodeBuddy 存储目录：`~/.codebuddy/projects/<cwd 中 '/' 换 '-'>`
/// （spec §1：与 Claude 的目录结构平行）。
pub fn codebuddy_project_dir(cwd: &Path) -> PathBuf {
    home_dir().join(".codebuddy").join("projects").join(project_key(cwd))
}

/// cwd → dozer 自己为 OpenCode 代写的 transcript 目录（OpenCode 本身没有
/// JSONL 落盘，dozer-hook 按 Claude 格式代写；spec §5.3）。
pub fn opencode_project_dir(cwd: &Path) -> PathBuf {
    home_dir()
        .join(".dozer")
        .join("agents")
        .join("opencode")
        .join("projects")
        .join(project_key(cwd))
}

/// transcript 首段 → 首句人类发言(首个字符串型 user content)。纯函数。
pub fn conversation_title(jsonl_head: &str) -> Option<String> {
    for line in jsonl_head.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("user")
            && let Some(text) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
        {
            return Some(text.to_string());
        }
    }
    None
}

/// 该对话是否是当前活会话(其路径在打开着的 transcript 集合中)。纯函数。
pub fn is_current_conversation(meta_path: &Path, open_transcripts: &[String]) -> bool {
    let p = meta_path.to_string_lossy();
    open_transcripts.iter().any(|o| o.as_str() == p)
}

/// 列出目录下所有 .jsonl 为对话(mtime 倒序)。读失败/非目录返回空。
/// 标题只读文件前若干字节以省 IO。`agent` 用于给每条记录打标签，不影响
/// 扫描逻辑本身。
pub fn list_conversations(agent: AgentKind, dir: &Path) -> Vec<ConversationMeta> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<ConversationMeta> = rd
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                return None;
            }
            let meta = e.metadata().ok()?;
            let size_bytes = meta.len();
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let head = read_head(&path, 16 * 1024);
            let title = conversation_title(&head).unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "(无标题对话)".into())
            });
            Some(ConversationMeta {
                path,
                title,
                modified_ms,
                size_bytes,
                agent,
            })
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.modified_ms));
    out
}

/// 合并 Claude/CodeBuddy/OpenCode 三个目录下同一个 cwd 的历史对话，按
/// mtime 统一倒序（spec §7：历史侧栏展示"这个项目下所有对话"，不管当年
/// 用哪个 agent 跑的）。
pub fn list_all_conversations(cwd: &Path) -> Vec<ConversationMeta> {
    let mut out = list_conversations(AgentKind::Claude, &claude_project_dir(cwd));
    out.extend(list_conversations(
        AgentKind::Codebuddy,
        &codebuddy_project_dir(cwd),
    ));
    out.extend(list_conversations(
        AgentKind::Opencode,
        &opencode_project_dir(cwd),
    ));
    out.sort_by_key(|m| std::cmp::Reverse(m.modified_ms));
    out
}

/// 读文件前 n 字节为 String(lossy)。
fn read_head(path: &Path, n: usize) -> String {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..read]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_first_string_user_turn() {
        let head = concat!(
            "{\"type\":\"mode\",\"mode\":\"x\"}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n"
        );
        assert_eq!(conversation_title(head).as_deref(), Some("改一下 README"));
        assert_eq!(conversation_title("{\"type\":\"mode\"}\nbad\n"), None);
    }

    #[test]
    fn is_current_matches_open_transcript() {
        use std::path::Path;
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation(Path::new("/t/a.jsonl"), &opens));
        assert!(!is_current_conversation(Path::new("/t/c.jsonl"), &opens));
    }

    #[test]
    fn project_dir_maps_slashes_to_dashes() {
        let d = claude_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.claude/projects/-a-b-c"));
    }

    #[test]
    fn codebuddy_dir_uses_codebuddy_root() {
        let d = codebuddy_project_dir(std::path::Path::new("/a/b/c"));
        assert!(d.to_string_lossy().ends_with("/.codebuddy/projects/-a-b-c"));
    }

    #[test]
    fn opencode_dir_lives_under_dozer_data_dir() {
        let d = opencode_project_dir(std::path::Path::new("/a/b/c"));
        assert!(
            d.to_string_lossy()
                .ends_with("/.dozer/agents/opencode/projects/-a-b-c")
        );
    }

    #[test]
    fn list_sorts_by_mtime_desc_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("one.jsonl");
        let p2 = dir.path().join("two.jsonl");
        std::fs::write(
            &p1,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第一个\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            &p2,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二个\"}}\n",
        )
        .unwrap();
        let list = list_conversations(AgentKind::Claude, dir.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "第二个", "mtime 倒序:后写的在前");
        assert_eq!(list[0].agent, AgentKind::Claude);
        assert!(list[0].size_bytes > 0);
        assert!(
            list_conversations(AgentKind::Claude, std::path::Path::new("/no/such/dir")).is_empty()
        );
    }

    #[test]
    fn list_conversations_tags_requested_agent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();
        let list = list_conversations(AgentKind::Codebuddy, dir.path());
        assert_eq!(list[0].agent, AgentKind::Codebuddy);
    }

    #[test]
    fn list_all_conversations_merges_three_dirs_sorted_by_mtime() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: 测试串行执行，临时改 HOME 后立即恢复；claude/codebuddy/opencode
        // 三个目录解析函数都读 HOME 环境变量。
        let prev_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", home.path()) };

        let cwd = std::path::Path::new("/proj");
        let claude_dir = claude_project_dir(cwd);
        let codebuddy_dir = codebuddy_project_dir(cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codebuddy_dir).unwrap();
        std::fs::write(
            claude_dir.join("a.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"claude 对话\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            codebuddy_dir.join("b.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"codebuddy 对话\"}}\n",
        )
        .unwrap();

        let list = list_all_conversations(cwd);

        match prev_home {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].agent, AgentKind::Codebuddy, "后写入的在前");
        assert_eq!(list[1].agent, AgentKind::Claude);
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app conversation:: -- --nocapture`
Expected: PASS（9 个测试：2 个原有 + 7 个新增/改写）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/conversation.rs
git commit -m "feat(dozer-app): agent-aware conversation directory resolution and merged history listing"
```

---

## Task 10: `dozer-app::transcript` 按 agent 分派解析

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs`（整份文件）

**Interfaces:**
- Consumes: `AgentKind`（Task 1）。
- Produces: `pub fn parse_transcript(agent: AgentKind, jsonl: &str) -> Vec<ReviewEntry>`（`Claude`/`Opencode` 复用同一套解析；`Codebuddy`/`Unknown` 返回空，等 CodeBuddy 适配计划补齐）。

- [ ] **Step 1: 写失败的测试**

替换/新增 `transcript.rs` 测试模块：

```rust
    #[test]
    fn parses_human_and_ai_turn_skipping_noise() {
        let jsonl = r#"
{"type":"mode","mode":"x"}
{"type":"user","message":{"role":"user","content":"改一下 README"}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"好的，我来改。"},{"type":"tool_use","name":"Edit","input":{"file_path":"/r/README.md"}},{"type":"tool_use","name":"Bash","input":{"command":"cargo test --all"}}]}}
不是 json 的坏行
{"type":"attachment","attachment":{}}
"#;
        let entries = parse_transcript(AgentKind::Claude, jsonl);
        assert_eq!(
            entries.len(),
            2,
            "1 人类 + 1 AI 回合;工具结果/噪音/坏行跳过"
        );
        assert_eq!(
            entries[0],
            ReviewEntry::Human {
                text: "改一下 README".into()
            }
        );
        match &entries[1] {
            ReviewEntry::AiTurn {
                text,
                tools,
                thinking,
            } => {
                assert_eq!(text, "好的，我来改。");
                assert!(*thinking);
                assert_eq!(
                    tools,
                    &vec![
                        "Edit README.md".to_string(),
                        "Bash cargo test --all".to_string()
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_and_all_noise_yield_nothing() {
        assert!(parse_transcript(AgentKind::Claude, "").is_empty());
        assert!(parse_transcript(AgentKind::Claude, "{\"type\":\"mode\"}\nbad\n").is_empty());
    }

    #[test]
    fn opencode_reuses_claude_shaped_parser() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"opencode 里也这么解析"}}"#;
        let entries = parse_transcript(AgentKind::Opencode, jsonl);
        assert_eq!(
            entries,
            vec![ReviewEntry::Human {
                text: "opencode 里也这么解析".into()
            }]
        );
    }

    #[test]
    fn codebuddy_and_unknown_yield_empty_until_schema_confirmed() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"应该被忽略"}}"#;
        assert!(parse_transcript(AgentKind::Codebuddy, jsonl).is_empty());
        assert!(parse_transcript(AgentKind::Unknown, jsonl).is_empty());
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app transcript:: -- --nocapture`
Expected: FAIL（`parse_transcript` 签名不匹配，编译错误）。

- [ ] **Step 3: 实现**

`transcript.rs` 顶部 `use` 加：

```rust
use dozer_core::protocol::AgentKind;
```

把现有 `pub fn parse_transcript(jsonl: &str) -> Vec<ReviewEntry> { ... }`（第 48-111 行）改名为私有函数 `parse_claude_shaped_jsonl`，函数体不变：

```rust
fn parse_claude_shaped_jsonl(jsonl: &str) -> Vec<ReviewEntry> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        // ...函数体逐字不变...
    }
    out
}
```

在其上方新增公开入口：

```rust
/// 按 agent 分派 transcript 解析。`Opencode` 复用 Claude 分支——
/// dozer-hook 代写 OpenCode 的 transcript 时就是按 Claude 字段形状写的
/// （spec §5.3），不是巧合。`Codebuddy`/`Unknown` 暂时返回空：CodeBuddy
/// 真实 transcript schema 待独立的适配计划验证后再接（spec §6），在那之前
/// "不产出数据"是唯一诚实的行为，不是占位符。
pub fn parse_transcript(agent: AgentKind, jsonl: &str) -> Vec<ReviewEntry> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode => parse_claude_shaped_jsonl(jsonl),
        AgentKind::Codebuddy | AgentKind::Unknown => Vec::new(),
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app transcript:: -- --nocapture`
Expected: PASS（4 个测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "feat(dozer-app): dispatch transcript parsing by AgentKind"
```

---

## Task 11: `dozer-app::workspace` 接线（`SessionTab`/`Message`/调用点）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:740`（`Message::AgentStateChanged` 变体签名）
- Modify: `crates/dozer-app/src/workspace.rs:981-990`（`SessionTab` 结构体新增字段）
- Modify: `crates/dozer-app/src/workspace.rs:1337-1349`（第一处 `SessionTab` 构造）
- Modify: `crates/dozer-app/src/workspace.rs:1939-1952`（第二处 `SessionTab` 构造，`on_tab_attached`）
- Modify: `crates/dozer-app/src/workspace.rs:1500-1506`（`spawn_conversations_refresh`，改用 `list_all_conversations`）
- Modify: `crates/dozer-app/src/workspace.rs:1549-1564`（`spawn_review_load` 签名新增 `agent` 参数）
- Modify: `crates/dozer-app/src/workspace.rs:2657-2666`（`Message::AgentStateChanged` 处理分支）
- Modify: `crates/dozer-app/src/workspace.rs:2710-2721`（回合结束重解析 transcript 调用点）
- Modify: `crates/dozer-app/src/workspace.rs:2893-2910`（`Message::ConversationOpen` 处理分支）
- Modify: `crates/dozer-app/src/workspace.rs:3649-3652`（`forward_events` 里 `TermEvent::Agent` → `Message` 转换）
- Modify: `crates/dozer-app/src/workspace.rs:4213,4216`（`conversation_sub` 调用点，改传 `.label()`）
- Modify: `crates/dozer-app/src/workspace.rs:6690,6693`（`conversation_sub_line_format` 测试，字符串字面量不变，仅确认签名未变）

**Interfaces:**
- Consumes: Task 4（`Reply::AgentEvent.agent`）、Task 8（`TermEvent::Agent.agent`）、Task 9（`ConversationMeta.agent: AgentKind`、`list_all_conversations`）、Task 10（`parse_transcript(agent, jsonl)`）、`SessionInfo.agent`（Task 2）。
- Produces: `SessionTab.agent: AgentKind`（随每次 `AgentEvent` 更新，初值来自 `SessionInfo.agent`）；GUI 端到端能正确解析任意 agent 来源的 transcript（本计划范围内 Codebuddy 分支解析结果恒空，行为已在 Task 10 锁定）。

本任务是纯类型穿线 + 少量调用点改写，不引入新的可观察行为分支（`AgentKind::Codebuddy`/`Unknown` 情况下 UI 该怎么样还怎么样，只是现在能拿到正确的值），因此不新增 iced 相关的单元测试，靠 `cargo build`/`cargo test -p dozer-app` 做回归验证——这与 workspace.rs 里同类"结构体加字段、贯穿现有调用链"的既有改动（如 P2a 加 `project_id`）验证方式一致。

- [ ] **Step 1: 确认改动前基线通过**

Run: `cargo test -p dozer-app -- --nocapture`
Expected: 在完成 Task 1-10 后，此刻 `dozer-app` 应该编译失败（`Message::AgentStateChanged` 构造点、`SessionTab` 构造点、`parse_transcript`/`conversation_sub` 调用点都还没跟上新签名）——这是预期状态，本任务就是来修的。留证据："Task 10 交付时 dozer-core/dozerd/dozer-client/dozer-hook 全绿，只有 dozer-app 因为接口变了而红"。

- [ ] **Step 2: 逐点改写**

`Message::AgentStateChanged` 变体（第 740 行）：

```rust
    AgentStateChanged(ProjectId, usize, AgentKind, AgentState, Option<String>),
```

第 48 行的顶部 import 加上 `AgentKind`：

```rust
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};
```

`SessionTab` 结构体（第 981-990 行，紧跟 `agent_state` 字段之后）：

```rust
    /// 会话内 agent 的最新状态（hook 事件驱动；初值来自
    /// `SessionInfo.agent_state`，晚 attach 也能恢复现状）。
    pub agent_state: AgentState,
    /// 会话归属的 agent（P2b；初值来自 `SessionInfo.agent`，随
    /// `AgentStateChanged` 更新）。
    pub agent: AgentKind,
    /// 该会话的 transcript 路径（有则终端 tab 显"审阅"入口；P1i）。
    pub transcript_path: Option<String>,
```

第一处构造（第 1337-1349 行，`ProjectRestore` 恢复路径）：

```rust
            tabs.push(SessionTab {
                agent_state: info.agent_state,
                agent: info.agent,
                transcript_path: info.transcript_path.clone(),
                info,
                model,
                alive: true,
                tab_id,
                forwarder,
                osc: OscScanner::new(),
                cwd: None,
                last_exit: None,
                delivery_pending: false,
                last_turn_head: None,
```

第二处构造（第 1939-1952 行，`on_tab_attached`）：

```rust
        self.tabs.push(SessionTab {
            agent_state: info.agent_state,
            agent: info.agent,
            transcript_path: info.transcript_path.clone(),
            info,
            model,
            alive: true,
            tab_id,
            forwarder,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            delivery_pending: false,
            last_turn_head: None,
        });
```

`spawn_conversations_refresh`（第 1500-1506 行）：

```rust
        io.handle.spawn(async move {
            let list = tokio::task::spawn_blocking(move || conversation::list_all_conversations(&cwd))
                .await
                .unwrap_or_default();
            tracing::debug!(n = list.len(), "对话列表扫描完成");
            let _ = proxy.send_event(Message::ConversationsRefreshed(project_id, list));
        });
```

（这处删掉了原来的 `let dir = conversation::claude_project_dir(&cwd);` 和把 `cwd`/日志字段直接传给 `tracing::debug!` 的写法——`list_all_conversations` 自己在闭包内部按 cwd 扫三个目录，日志里保留原有的 `n = list.len()` 字段即可，`cwd = %cwd.display()` 字段因为 `cwd` 被 move 进闭包、拿不到外层引用了，直接删掉那个字段而不是额外 clone，符合 YAGNI。）

`spawn_review_load` 签名与调用（第 1549-1564 行）：

```rust
    /// 异步读 transcript + 解析 → ReviewLoaded（GUI 侧 spawn_blocking；P1i/P1j/P2b 按源）。
    fn spawn_review_load(&self, io: &ShellIo, source: ReviewSource, path: String, agent: AgentKind) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                std::fs::read_to_string(&path)
                    .map(|s| transcript::parse_transcript(agent, &s))
                    .map_err(|e| format!("无法读取会话记录: {e}"))
            })
            .await
            .unwrap_or_else(|e| Err(format!("解析任务失败: {e}")));
            let _ = proxy.send_event(Message::ReviewLoaded(project_id, source, result));
        });
    }
```

`Message::AgentStateChanged` 处理分支（第 2657-2666 行）：

```rust
            Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
                self.with_project(project_id, |ws, io| {
                    // 当前项目路径先取出（下面要 &mut 借 tab，冲突）；重锚:项目优先。
                    let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
                    if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                        tab.agent_state = state;
                        tab.agent = agent;
                        if let Some(tp) = transcript_path {
                            tab.transcript_path = Some(tp);
                        }
                        tracing::info!(tab_id, ?state, "agent 状态变更");
```

（后续 `if state == AgentState::TurnEnded { ... }` 那一大段不变。）

回合结束重解析 transcript 调用点（第 2710-2721 行）：

```rust
                    // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
                    if state == AgentState::TurnEnded
                        && let Some(rv) = &ws.review
                        && review_should_refresh_on_turn(&rv.source, tab_id)
                        && let Some((path, tab_agent)) = ws
                            .tabs
                            .iter()
                            .find(|t| t.tab_id == tab_id)
                            .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
                    {
                        ws.spawn_review_load(io, ReviewSource::Session(tab_id), path, tab_agent);
                    }
```

`Message::ConversationOpen` 处理分支（第 2893-2910 行）：

```rust
            Message::ConversationOpen(path) => {
                self.with_focused_project(move |ws, io| {
                    // 若点开的是某活会话的当前对话 → Session 源(回合结束刷新);否则 File 快照。
                    let path_s = path.to_string_lossy().into_owned();
                    let session_tab = ws
                        .tabs
                        .iter()
                        .find(|t| t.transcript_path.as_deref() == Some(path_s.as_str()));
                    let agent = session_tab
                        .map(|t| t.agent)
                        .or_else(|| {
                            ws.conversations
                                .iter()
                                .find(|c| c.path == path)
                                .map(|c| c.agent)
                        })
                        .unwrap_or_default();
                    let source = session_tab
                        .map(|t| ReviewSource::Session(t.tab_id))
                        .unwrap_or_else(|| ReviewSource::File(path.clone()));
                    ws.review = Some(ReviewView {
                        source: source.clone(),
                        entries: Vec::new(),
                        error: None,
                        expanded: std::collections::HashSet::new(),
                    });
                    ws.spawn_review_load(io, source, path_s, agent);
                });
            }
```

`forward_events` 里 `TermEvent::Agent` 转换（第 3649-3652 行）：

```rust
            TermEvent::Agent {
                agent,
                state,
                transcript_path,
            } => Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path),
```

`conversation_sub` 调用点（第 4213、4216 行），`&c.agent` 换成 `c.agent.label()`：

```rust
        let sub = if current {
            format!(
                "● 当前 · {}",
                conversation_sub(c.agent.label(), c.modified_ms, c.size_bytes, now_ms)
            )
        } else {
            conversation_sub(c.agent.label(), c.modified_ms, c.size_bytes, now_ms)
        };
```

（`conversation_sub` 自身签名 `fn conversation_sub(agent: &str, ...)` 不变，`.label()` 返回 `&'static str`，调用点类型对得上，第 6690/6693 行的既有测试 `conversation_sub("claude", ...)` 不需要改。）

- [ ] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozer-app -- --nocapture`
Expected: PASS，全量 `dozer-app` 测试绿（含 Task 9/10 新增测试的间接消费方）。

- [ ] **Step 4: 全 workspace 编译 + 测试确认无回归**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 全部 crate 编译通过、测试全绿。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): thread AgentKind through SessionTab, Message::AgentStateChanged, and review/conversation call sites"
```

---

## 完成检查（对照 spec 的"本计划范围"）

- [ ] `AgentKind` 存在于协议层，三个消息类型都带上了，老协议帧回落 `Unknown`。
- [ ] `dozerd` 端到端：`Request::HookEvent.agent` → `Session.agent` → `Reply::AgentEvent.agent`，有集成测试覆盖。
- [ ] `dozer-hook` 装出来的 Claude hook command 形如 `<exe> claude <event>`，转发时 `agent = Claude`。
- [ ] `dozer-app` 的 transcript 解析器、对话历史扫描器都已经是"按 `AgentKind` 分派"的形状，Claude 分支行为逐字节不变，Codebuddy 分支诚实地返回空（不是编译时占位符，是运行时真实、已测试的降级路径）。
- [ ] `cargo build --workspace && cargo test --workspace` 全绿。
- [ ] 没有加任何"新建会话选 agent"的 UI（不在本计划范围）。

后续两个独立计划（CodeBuddy 适配、OpenCode 适配）都依赖本计划已合并的 `AgentKind`/`parse_transcript(agent, ...)`/`project_dir` 系列函数，但互相不依赖，可以任意顺序推进。
