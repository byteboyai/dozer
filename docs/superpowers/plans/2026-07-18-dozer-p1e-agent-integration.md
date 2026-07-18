# Dozer P1e agent 集成基础层 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** OSC 7/133 shell 集成 + Claude Code hook 事件通路（dozer-hook → dozerd → GUI）+ 终端 tab 四态 agent 状态胶囊 + `dozer-hook install` 注册命令。

**Architecture:** OSC 解析是 dozer-app 侧的有状态字节扫描器（观察不剥离——alacritty 静默忽略未知 OSC，与 spec §3"剥离"措辞的偏差记录于 Task 4，行为等价）；hook 事件复用既有 UDS + JSON Lines 协议新增 `Request::HookEvent`，经 session 的 broadcast 通道随 attach 流下发 `Reply::AgentEvent`；最新状态记在 Session 内、随 `SessionInfo.agent_state` 供晚 attach 恢复；zsh 发射端由 dozerd 经 ZDOTDIR 包装注入。

**Tech Stack:** Rust workspace（dozer-core/dozerd/dozer-client/dozer-app/dozer-hook）、tokio、serde JSON Lines、alacritty_terminal 0.26、iced 0.14、std::os::unix::net（hook 侧阻塞 UDS）。

**Spec:** `docs/superpowers/specs/2026-07-18-dozer-p1e-agent-integration-design.md`

## Global Constraints

- 主题 ByteBoy2077：`theme::GREEN`=运行、`theme::PURPLE`=待输入、`theme::GOLD`=回合结束（金=甲方动作专属）、`theme::RED`=错误提示、`theme::DIM`=非活。
- 协议向后兼容：`SessionInfo` 新字段必须 `#[serde(default)]`，旧客户端 JSON 可解。
- dozer-hook 绝不拖慢 agent：任何失败静默退出码 0（install/uninstall 除外），UDS 超时 200ms。
- dozerd 保持哑字节管道：不解析 OSC，不上 SQLite。
- 测试全部 headless（P1b-d 惯例）；GUI 行为无法 headless 的部分归 Task 7 人工验收。
- commit 信息中文、结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。
- 每个 Task 完成前跑 `cargo clippy --all-targets && cargo fmt`，零警告。

---

### Task 1: dozer-core 协议扩展（AgentState + HookEvent/AgentEvent）

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozerd/src/session.rs`（`info()` 补新字段，仅编译修复）

**Interfaces:**
- Produces:
  - `AgentState`（`Idle|Running|AwaitingInput|TurnEnded`，`Copy`，`Default=Idle`，serde snake_case）
  - `SessionInfo.agent_state: AgentState`（`#[serde(default)]`）
  - `Request::HookEvent { session_id: String, event: String, ts_ms: u64, data: serde_json::Value }`
  - `Reply::AgentEvent { session_id: String, state: AgentState, event: String, ts_ms: u64 }`

- [x] **Step 1: 写失败测试**（`protocol.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn hook_event_roundtrips() {
        let req = Request::HookEvent {
            session_id: "s1".into(),
            event: "Stop".into(),
            ts_ms: 123,
            data: serde_json::json!({"transcript_path": "/tmp/t.jsonl"}),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn agent_event_reply_tags_snake_case() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s1".into(),
            state: AgentState::AwaitingInput,
            event: "Notification".into(),
            ts_ms: 5,
        });
        assert!(line.contains(r#""type":"agent_event""#));
        assert!(line.contains(r#""state":"awaiting_input""#));
    }

    #[test]
    fn old_session_info_without_agent_state_decodes_as_idle() {
        // P1b-d 时代的 SessionInfo JSON（无 agent_state 字段）必须可解
        let old = r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.agent_state, AgentState::Idle);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-core`
Expected: 编译错误（`AgentState` 未定义、`HookEvent`/`AgentEvent` 变体不存在）。

- [x] **Step 3: 最小实现**

`protocol.rs`：`SessionInfo` 上方加：

```rust
/// agent 会话状态（hook 事件驱动的四态机；spec P1e D6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    #[default]
    Idle,
    Running,
    AwaitingInput,
    TurnEnded,
}
```

`SessionInfo` 末尾加字段：

```rust
    /// 会话内 agent 的最新状态；旧协议帧无此字段时回落 Idle。
    #[serde(default)]
    pub agent_state: AgentState,
```

`Request` 追加变体：

```rust
    /// dozer-hook 单向上报的 agent hook 事件；data 原样透传（P1f 消费）。
    HookEvent {
        session_id: String,
        event: String,
        ts_ms: u64,
        data: serde_json::Value,
    },
```

`Reply` 追加变体：

```rust
    /// hook 事件引起的状态变更，随 attach 流广播给该会话的订阅者。
    AgentEvent {
        session_id: String,
        state: AgentState,
        event: String,
        ts_ms: u64,
    },
```

`crates/dozerd/src/session.rs` 的 `info()`（构造 `SessionInfo` 处）补：

```rust
            agent_state: AgentState::default(),
```

（顶部 `use dozer_core::protocol::SessionInfo;` 改为 `use dozer_core::protocol::{AgentState, SessionInfo};`。Task 2 会把这里换成真实状态。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-core && cargo build --workspace`
Expected: 新增 3 测试通过，全 workspace 编译通过（server.rs 的 match 若报 non-exhaustive，在 `Request` 的 match 里临时加 `Request::HookEvent { .. } => Reply::Ok,` 占位分支——Task 2 替换为真实实现）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-core crates/dozerd
git commit -m "feat(协议): AgentState 四态 + HookEvent/AgentEvent 消息——P1e hook 通路契约

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: dozerd hook 事件入站 + 状态存取 + DOZER_SESSION_ID 注入

**Files:**
- Modify: `crates/dozerd/src/session.rs`
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: Task 1 全部类型。
- Produces:
  - `SessionEvent::Agent { state: AgentState, event: String, ts_ms: u64 }`（broadcast 通道新变体）
  - `Session::set_agent_state(&self, state: AgentState, event: &str, ts_ms: u64)`
  - `server::agent_state_for(event: &str) -> Option<AgentState>`（纯函数）
  - 会话进程环境变量 `DOZER_SESSION_ID=<session id>`
  - `Request::HookEvent` → 记状态 + 广播 + `Reply::Ok`（未知会话也 `Ok`，仅日志）

- [x] **Step 1: 写失败测试**

`session.rs` 的 `mod tests` 追加：

```rust
    #[tokio::test]
    async fn spawn_injects_dozer_session_id_env() {
        let s = Session::spawn(SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "echo id=$DOZER_SESSION_ID; sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        })
        .unwrap();
        let needle = format!("id={}", s.id());
        let mut found = false;
        for _ in 0..100 {
            if String::from_utf8_lossy(&s.snapshot().0).contains(&needle) {
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        assert!(found, "PTY 输出应含注入的会话 id");
        let _ = s.kill();
    }

    #[tokio::test]
    async fn set_agent_state_updates_info_and_broadcasts() {
        let s = Session::spawn(SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        })
        .unwrap();
        let mut rx = s.subscribe();
        s.set_agent_state(AgentState::Running, "UserPromptSubmit", 42);
        assert_eq!(s.info().agent_state, AgentState::Running);
        loop {
            match rx.recv().await.unwrap() {
                SessionEvent::Agent { state, event, ts_ms } => {
                    assert_eq!(state, AgentState::Running);
                    assert_eq!(event, "UserPromptSubmit");
                    assert_eq!(ts_ms, 42);
                    break;
                }
                _ => continue, // PTY 启动输出等无关事件
            }
        }
        let _ = s.kill();
    }
```

`server.rs` 的测试（现有集成测试文件在哪就加哪；若 server.rs 无 `mod tests` 则新建）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_state_mapping_matches_spec_d6() {
        use dozer_core::protocol::AgentState::*;
        assert_eq!(agent_state_for("UserPromptSubmit"), Some(Running));
        assert_eq!(agent_state_for("PreToolUse"), Some(Running));
        assert_eq!(agent_state_for("PostToolUse"), Some(Running));
        assert_eq!(agent_state_for("Notification"), Some(AwaitingInput));
        assert_eq!(agent_state_for("Stop"), Some(TurnEnded));
        assert_eq!(agent_state_for("SessionStart"), Some(Idle));
        assert_eq!(agent_state_for("SessionEnd"), Some(Idle));
        assert_eq!(agent_state_for("SomethingNew"), None);
    }
}
```

另在 `crates/dozerd/tests/`（若已有集成测试文件沿用其命名，否则建 `hook_events.rs`）加端到端测试：起 `serve` 于临时 socket，连接 A attach 一个会话，连接 B 发 `HookEvent`，断言 A 收到 `Reply::AgentEvent` 且随后的 `ListSessions` 里 `agent_state == TurnEnded`；再对不存在的 session_id 发 `HookEvent`，断言得 `Reply::Ok`：

```rust
use dozer_core::protocol::{AgentState, Reply, Request, decode_line, encode_line};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

async fn send_req(sock: &std::path::Path, req: &Request) -> Reply {
    let stream = UnixStream::connect(sock).await.unwrap();
    let (r, mut w) = stream.into_split();
    w.write_all(encode_line(req).as_bytes()).await.unwrap();
    let mut lines = BufReader::new(r).lines();
    decode_line(&lines.next_line().await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn hook_event_reaches_attached_client_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    let registry = Arc::new(SessionRegistry::new());
    let sock2 = sock.clone();
    let reg2 = registry.clone();
    tokio::spawn(async move { dozerd::server::serve(&sock2, reg2).await.unwrap() });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let created = match send_req(&sock, &Request::CreateSession {
        name: "t".into(), command: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 5".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80, rows: 24,
    }).await {
        Reply::Created { session } => session,
        other => panic!("{other:?}"),
    };

    // 连接 A：attach 并持流
    let stream = UnixStream::connect(&sock).await.unwrap();
    let (r, mut w) = stream.into_split();
    w.write_all(encode_line(&Request::Attach { session_id: created.id.clone(), from_offset: 0 }).as_bytes())
        .await.unwrap();
    let mut lines = BufReader::new(r).lines();
    let _attached = lines.next_line().await.unwrap().unwrap();

    // 连接 B：hook 事件
    match send_req(&sock, &Request::HookEvent {
        session_id: created.id.clone(), event: "Stop".into(), ts_ms: 7,
        data: serde_json::Value::Null,
    }).await {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }

    // A 在 PTY 噪音里等到 AgentEvent
    loop {
        let line = tokio::time::timeout(
            std::time::Duration::from_secs(5), lines.next_line(),
        ).await.unwrap().unwrap().unwrap();
        if let Ok(Reply::AgentEvent { state, event, .. }) = decode_line::<Reply>(&line) {
            assert_eq!(state, AgentState::TurnEnded);
            assert_eq!(event, "Stop");
            break;
        }
    }

    match send_req(&sock, &Request::ListSessions).await {
        Reply::Sessions { sessions } => {
            assert_eq!(sessions[0].agent_state, AgentState::TurnEnded)
        }
        other => panic!("{other:?}"),
    }

    // 未知会话：Ok 不报错
    match send_req(&sock, &Request::HookEvent {
        session_id: "ghost".into(), event: "Stop".into(), ts_ms: 8,
        data: serde_json::Value::Null,
    }).await {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
}
```

（若 `dozerd` 的 `registry`/`server` 模块未 `pub` 导出到 lib，本任务顺手在 `crates/dozerd/src/lib.rs`—若无则新建—`pub mod` 暴露 `server`/`registry`/`session`/`ring`，`main.rs` 改 `use dozerd::…`；dev-dependencies 需要 `tempfile`。）

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd`
Expected: 编译错误（`set_agent_state`/`SessionEvent::Agent`/`agent_state_for` 不存在）。

- [x] **Step 3: 最小实现**

`session.rs`：

```rust
// SessionEvent 追加变体
    Agent {
        state: AgentState,
        event: String,
        ts_ms: u64,
    },
```

`Session` 结构体加字段 `agent_state: Mutex<AgentState>,`（`spawn` 的 `Ok(Self { ... })` 里初始化 `agent_state: Mutex::new(AgentState::default()),`）。

`spawn` 里 id 提前生成并注入 env（`CommandBuilder` 构造段）：

```rust
        let id = uuid::Uuid::new_v4().to_string();
        // ……CommandBuilder 各 env 之后：
        cmd.env("DOZER_SESSION_ID", &id);
```

（`Ok(Self { id: uuid::Uuid::new_v4().to_string(), … })` 改为 `Ok(Self { id, … })`。）

`impl Session` 加方法：

```rust
    /// hook 事件驱动的状态更新：记最新态 + 广播给本会话订阅者。
    pub fn set_agent_state(&self, state: AgentState, event: &str, ts_ms: u64) {
        *self.agent_state.lock().expect("agent_state lock") = state;
        let _ = self.tx.send(SessionEvent::Agent {
            state,
            event: event.to_string(),
            ts_ms,
        });
    }
```

`info()` 的 `agent_state: AgentState::default(),` 改为：

```rust
            agent_state: *self.agent_state.lock().expect("agent_state lock"),
```

`server.rs`：文件级加纯函数：

```rust
/// spec P1e D6：hook 事件名 → 四态映射；未知事件不改状态。
pub fn agent_state_for(event: &str) -> Option<dozer_core::protocol::AgentState> {
    use dozer_core::protocol::AgentState::*;
    match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(Running),
        "Notification" => Some(AwaitingInput),
        "Stop" => Some(TurnEnded),
        "SessionStart" | "SessionEnd" => Some(Idle),
        _ => None,
    }
}
```

Task 1 的占位分支替换为：

```rust
                        Request::HookEvent { session_id, event, ts_ms, data: _ } => {
                            match registry.get(&session_id) {
                                None => {
                                    tracing::debug!(%session_id, %event, "hook 事件的会话不存在，丢弃");
                                }
                                Some(s) => match agent_state_for(&event) {
                                    Some(state) => s.set_agent_state(state, &event, ts_ms),
                                    None => tracing::debug!(%event, "未知 hook 事件，不改状态"),
                                },
                            }
                            Reply::Ok
                        }
```

attach 转发 select 臂（`Ok(SessionEvent::Exited …)` 之前）追加：

```rust
                    Ok(SessionEvent::Agent { state, event, ts_ms }) => {
                        let reply = Reply::AgentEvent { session_id: sid, state, event, ts_ms };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                    }
```

（注意：`Agent` 事件不参与 `sent_until` 水位——水位只治 `Output` 字节流。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd`
Expected: 全绿，含新增 4 个测试。

- [x] **Step 5: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): hook 事件入站——四态映射 + 会话状态存取 + AgentEvent 广播 + DOZER_SESSION_ID 注入

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: dozer-client 透传 + workspace 状态胶囊

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 1 `AgentState`、`Reply::AgentEvent`、`SessionInfo.agent_state`。
- Produces:
  - `TermEvent::Agent(AgentState)`（client 事件新变体）
  - `Message::AgentStateChanged(usize, AgentState)`（tab_id 路由）
  - `SessionTab.agent_state: AgentState` 字段
  - `workspace::agent_badge(state: AgentState, alive: bool) -> Option<(&'static str, iced_widget::core::Color)>`（纯函数，Task 7 验收的胶囊形态）

- [x] **Step 1: 写失败测试**（`workspace.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn agent_badge_states() {
        use dozer_core::protocol::AgentState::*;
        assert_eq!(agent_badge(Idle, true), None, "Idle 不出胶囊");
        assert_eq!(agent_badge(Running, false), None, "死会话不出胶囊");
        let (label, color) = agent_badge(Running, true).unwrap();
        assert_eq!(label, "运行中");
        assert_eq!(color, theme::GREEN);
        let (label, color) = agent_badge(AwaitingInput, true).unwrap();
        assert_eq!(label, "待输入");
        assert_eq!(color, theme::PURPLE);
        let (label, color) = agent_badge(TurnEnded, true).unwrap();
        assert_eq!(label, "回合毕");
        assert_eq!(color, theme::GOLD);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app agent_badge`
Expected: 编译错误（`agent_badge` 未定义）。

- [x] **Step 3: 实现**

`dozer-client/src/lib.rs`：

```rust
// 头部 use 增 AgentState
use dozer_core::protocol::{AgentState, Reply, Request, SessionInfo, decode_line, encode_line};

// TermEvent 追加变体
    /// 本会话 agent 状态变更（hook 事件驱动，dozerd 广播）。
    Agent(AgentState),
```

attach 读循环的 decode match（`Ok(Reply::Exited …)` 之前）追加：

```rust
                                    Ok(Reply::AgentEvent { state, .. }) => TermEvent::Agent(state),
```

`dozer-app/src/workspace.rs`：

1. `use dozer_core::protocol::{AgentState, SessionInfo};`（原本只有 `SessionInfo`）。
2. `Message` 追加：

```rust
    /// attach 流转发来的 agent 状态变更（`usize` 是 tab 稳定 id）。
    AgentStateChanged(usize, AgentState),
```

3. `SessionTab` 加 pub 字段 `pub agent_state: AgentState,`；三处构造（`bootstrap`、`on_tab_attached`）初始化 `agent_state: info.agent_state,`（注意 `info` 被 move 进结构体，取值须在 move 之前或用结构体更新顺序规避）。
4. `update` 追加分支：

```rust
            Message::AgentStateChanged(tab_id, state) => {
                if let Some(tab) = self.tab_by_id_mut(tab_id) {
                    tab.agent_state = state;
                }
            }
```

5. `forward_events` 的 match 追加：

```rust
            TermEvent::Agent(state) => Message::AgentStateChanged(tab_id, state),
```

6. 文件级纯函数 + `tab_item` 接线：

```rust
/// tab 上的 agent 状态胶囊：Idle/死会话不出胶囊；四态配色见 Global Constraints。
fn agent_badge(state: AgentState, alive: bool) -> Option<(&'static str, Color)> {
    if !alive {
        return None;
    }
    match state {
        AgentState::Idle => None,
        AgentState::Running => Some(("运行中", theme::GREEN)),
        AgentState::AwaitingInput => Some(("待输入", theme::PURPLE)),
        AgentState::TurnEnded => Some(("回合毕", theme::GOLD)),
    }
}
```

`tab_item` 的 `label` 改为（保持现有状态点，胶囊缀在名称后）：

```rust
    let mut label = row![
        text("●").size(10).color(dot_color),
        text(tab.info.name.clone()).size(12).color(theme::CREAM),
    ]
    .spacing(4);
    if let Some((badge, color)) = agent_badge(tab.agent_state, tab.alive) {
        label = label.push(text(badge).size(10).color(color));
    }
```

（测试可见性：`agent_badge` 放在 `impl Workspace` 外的文件级私有函数即可，`mod tests` 同文件可达。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-client && cargo test -p dozer-app`
Expected: 全绿。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-client crates/dozer-app
git commit -m "feat(GUI): agent 状态胶囊——TermEvent::Agent 透传 + tab 四态徽标(绿/紫/金)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: OSC 7/133 扫描器 + cwd 标题 / 退出码提示

**Files:**
- Create: `crates/dozer-app/src/osc.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod osc;` 声明）
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `crate::assets::percent_decode(s: &str) -> Option<String>`（已 pub）。
- Produces:
  - `OscScanner::new()` / `OscScanner::feed(&mut self, bytes: &[u8]) -> Vec<OscEvent>`
  - `OscEvent::{Cwd(PathBuf), CmdStart, CmdExit(i32)}`
  - `SessionTab.cwd: Option<PathBuf>`、`SessionTab.last_exit: Option<i32>`、`SessionTab::ingest_osc(&mut self, bytes: &[u8])`
  - `workspace::tab_title(cwd: Option<&std::path::Path>, fallback: &str) -> String`（纯函数）

**设计注记（对 spec §3 的偏差）**：扫描器观察字节流但**不剥离** OSC 序列——alacritty_terminal 的 vte 解析器静默消费未知 OSC，不会渲染成可见字符；观察式实现免去流改写，行为与 spec 意图等价。

- [x] **Step 1: 写失败测试**（`osc.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_bel_and_st_terminators() {
        let mut s = OscScanner::new();
        let ev = s.feed(b"\x1b]7;file://mac.local/Users/c/proj\x07");
        assert_eq!(ev, vec![OscEvent::Cwd("/Users/c/proj".into())]);
        let ev = s.feed(b"\x1b]7;file://mac.local/tmp/a%20b\x1b\\");
        assert_eq!(ev, vec![OscEvent::Cwd("/tmp/a b".into())]);
    }

    #[test]
    fn osc133_command_marks() {
        let mut s = OscScanner::new();
        let ev = s.feed(b"\x1b]133;C\x07ls output\x1b]133;D;0\x07\x1b]133;D;127\x07");
        assert_eq!(
            ev,
            vec![OscEvent::CmdStart, OscEvent::CmdExit(0), OscEvent::CmdExit(127)]
        );
    }

    #[test]
    fn sequence_split_across_feeds_resumes() {
        let mut s = OscScanner::new();
        assert!(s.feed(b"\x1b]7;file://h/Us").is_empty());
        let ev = s.feed(b"ers/c\x07");
        assert_eq!(ev, vec![OscEvent::Cwd("/Users/c".into())]);
    }

    #[test]
    fn oversize_sequence_abandoned() {
        let mut s = OscScanner::new();
        let mut big = b"\x1b]7;file://h/".to_vec();
        big.extend(std::iter::repeat_n(b'x', 4096));
        big.extend(b"\x07\x1b]133;C\x07");
        let ev = s.feed(&big);
        assert_eq!(ev, vec![OscEvent::CmdStart], "超长序列放弃，后续序列不受损");
    }

    #[test]
    fn cjk_and_other_escapes_produce_nothing() {
        let mut s = OscScanner::new();
        assert!(s.feed("你好\x1b[31m红\x1b[0m\x1b]0;title\x07".as_bytes()).is_empty());
    }
}
```

`workspace.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn tab_title_prefers_cwd_basename() {
        use std::path::Path;
        assert_eq!(tab_title(Some(Path::new("/Users/c/proj")), "shell"), "proj");
        assert_eq!(tab_title(Some(Path::new("/")), "shell"), "/");
        assert_eq!(tab_title(None, "shell"), "shell");
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app`
Expected: 编译错误（`osc` 模块不存在、`tab_title` 未定义）。

- [x] **Step 3: 实现**

`crates/dozer-app/src/osc.rs`：

```rust
//! OSC 7（cwd）/ OSC 133（命令边界+退出码）观察式扫描器。
//!
//! 有状态：序列可跨 `feed` 调用截断续接。只观察不剥离——alacritty
//! 静默消费未知 OSC，透传不影响渲染。单序列超过 `OSC_CAP` 即放弃
//! （防恶意/损坏流撑爆内存），后续序列不受影响。

use std::path::PathBuf;

/// 单条 OSC 序列净荷上限（`]` 与终止符之间的字节数）。
const OSC_CAP: usize = 2048;

#[derive(Debug, PartialEq)]
pub enum OscEvent {
    /// OSC 7：`file://host/path`（path 已 percent-decode）。
    Cwd(PathBuf),
    /// OSC 133;C：命令开始执行。
    CmdStart,
    /// OSC 133;D;<code>：命令结束，附退出码（缺省 0）。
    CmdExit(i32),
}

enum State {
    Ground,
    Esc,
    Osc,
    /// OSC 内遇到 ESC：期待 `\`（ST 终止符）。
    OscEsc,
}

pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            buf: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut out = Vec::new();
        for &b in bytes {
            self.state = match self.state {
                State::Ground => {
                    if b == 0x1b {
                        State::Esc
                    } else {
                        State::Ground
                    }
                }
                State::Esc => {
                    if b == b']' {
                        self.buf.clear();
                        State::Osc
                    } else {
                        State::Ground
                    }
                }
                State::Osc => match b {
                    0x07 => {
                        self.finish(&mut out);
                        State::Ground
                    }
                    0x1b => State::OscEsc,
                    _ => {
                        if self.buf.len() >= OSC_CAP {
                            self.buf.clear();
                            State::Ground // 超长：放弃本序列
                        } else {
                            self.buf.push(b);
                            State::Osc
                        }
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(&mut out);
                    }
                    State::Ground
                }
            };
        }
        out
    }

    fn finish(&mut self, out: &mut Vec<OscEvent>) {
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        if let Some(rest) = s.strip_prefix("7;") {
            if let Some(url) = rest.strip_prefix("file://") {
                // host 段到第一个 '/' 为止；余下是路径
                if let Some(slash) = url.find('/') {
                    if let Some(path) = crate::assets::percent_decode(&url[slash..]) {
                        out.push(OscEvent::Cwd(PathBuf::from(path)));
                    }
                }
            }
        } else if let Some(rest) = s.strip_prefix("133;") {
            match rest.as_bytes().first() {
                Some(b'C') => out.push(OscEvent::CmdStart),
                Some(b'D') => {
                    let code = rest
                        .get(2..)
                        .and_then(|c| c.parse::<i32>().ok())
                        .unwrap_or(0);
                    out.push(OscEvent::CmdExit(code));
                }
                _ => {}
            }
        }
    }
}
```

`main.rs`：`mod preview;` 之后加 `mod osc;`。

`workspace.rs`：

1. `use crate::osc::{OscEvent, OscScanner};`、`use std::path::Path;`（若未有）。
2. `SessionTab` 加字段：

```rust
    /// OSC 扫描器（每 tab 独立，序列可跨 chunk）。
    osc: OscScanner,
    /// OSC 7 上报的当前目录；tab 标题优先显示其 basename。
    pub cwd: Option<PathBuf>,
    /// OSC 133;D 上报的最近命令退出码；非零时终端栏红字提示；
    /// 下一条命令开始（133;C）时清除。
    pub last_exit: Option<i32>,
```

（`bootstrap`/`on_tab_attached` 构造处初始化 `osc: OscScanner::new(), cwd: None, last_exit: None,`。）

3. `impl SessionTab`（新建 impl 块）：

```rust
impl SessionTab {
    /// 把一段会话输出送进 OSC 扫描器并落地状态（观察式，不改写字节）。
    fn ingest_osc(&mut self, bytes: &[u8]) {
        for ev in self.osc.feed(bytes) {
            match ev {
                OscEvent::Cwd(p) => self.cwd = Some(p),
                OscEvent::CmdStart => self.last_exit = None,
                OscEvent::CmdExit(code) => self.last_exit = Some(code),
            }
        }
    }
}
```

4. 接线三处：
   - `Message::TermOutput` 分支：`let responses = tab.model.feed(&bytes);` 之前加 `tab.ingest_osc(&bytes);`
   - `bootstrap` 恢复循环：`tabs.push(SessionTab { … });` 之后加：

```rust
                            if let Some(t) = tabs.last_mut() {
                                t.ingest_osc(&snapshot);
                            }
```

   - `on_tab_attached`：`self.tabs.push(SessionTab { … });` 之后加：

```rust
        if let Some(t) = self.tabs.last_mut() {
            t.ingest_osc(&snapshot);
        }
```

（`model.feed(&snapshot)` 用的是引用，`snapshot` 此处仍可用。）

5. 文件级纯函数 + UI 接线：

```rust
/// tab 标题：OSC 7 的 cwd basename 优先，无 cwd 回落会话名。
fn tab_title(cwd: Option<&Path>, fallback: &str) -> String {
    match cwd {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned()),
        None => fallback.to_string(),
    }
}
```

`tab_item` 里 `text(tab.info.name.clone())` 改为 `text(tab_title(tab.cwd.as_deref(), &tab.info.name))`。

`terminal_pane` 里 `daemon_error` 提示之后追加退出码提示：

```rust
    if let Some(tab) = ws.tabs.get(ws.active) {
        if let Some(code) = tab.last_exit {
            if code != 0 {
                content = content.push(text(format!("exit {code}")).size(11).color(theme::RED));
            }
        }
    }
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app`
Expected: 全绿（osc 5 测 + tab_title 1 测 + 既有）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(终端): OSC 7/133 观察式扫描器——cwd 标题跟随 + 命令退出码提示

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: zsh 集成注入（dozerd ZDOTDIR 包装）

**Files:**
- Create: `crates/dozerd/assets/zdotdir/.zshenv`
- Create: `crates/dozerd/assets/zdotdir/.zshrc`
- Create: `crates/dozerd/src/shell_integration.rs`
- Modify: `crates/dozerd/src/main.rs`（或 lib.rs 的 mod 声明处）
- Modify: `crates/dozerd/src/session.rs`

**Interfaces:**
- Consumes: `dozer_core::paths::state_dir()`。
- Produces:
  - `shell_integration::ensure_zdotdir() -> anyhow::Result<PathBuf>`（daemon 启动时调用，落盘包装文件，幂等）
  - spawn 注入：`ZDOTDIR=<wrapper>`、`DOZER_ORIG_ZDOTDIR=<用户原 ZDOTDIR，可空>`、`DOZER_ZDOTDIR_WRAPPER=<wrapper>`（仅当 command basename 为 `zsh` 且 daemon 环境 `DOZER_SHELL_INTEGRATION != "0"`）

- [x] **Step 1: 写资产文件**

`crates/dozerd/assets/zdotdir/.zshenv`：

```zsh
# Dozer shell integration —— .zshenv 阶段：转发用户原 .zshenv 后回到包装目录。
# 已知边界：用户 .zshenv 若自改 ZDOTDIR，会被此处覆盖（spec P1e D2）。
if [[ -n "$DOZER_ORIG_ZDOTDIR" ]]; then
  ZDOTDIR="$DOZER_ORIG_ZDOTDIR"
else
  unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"
ZDOTDIR="$DOZER_ZDOTDIR_WRAPPER"
```

`crates/dozerd/assets/zdotdir/.zshrc`：

```zsh
# Dozer shell integration —— .zshrc 阶段：转发用户原 .zshrc 后挂 OSC 7/133 发射钩子。
if [[ -n "$DOZER_ORIG_ZDOTDIR" ]]; then
  ZDOTDIR="$DOZER_ORIG_ZDOTDIR"
else
  unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshrc" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshrc"

autoload -Uz add-zsh-hook
__dozer_osc7() { builtin printf '\e]7;file://%s%s\e\\' "${HOST:-localhost}" "$PWD" }
__dozer_preexec() { builtin printf '\e]133;C\e\\' }
__dozer_precmd() {
  local code=$status
  builtin printf '\e]133;D;%s\e\\' "$code"
  __dozer_osc7
}
add-zsh-hook preexec __dozer_preexec
add-zsh-hook precmd __dozer_precmd
__dozer_osc7
```

- [x] **Step 2: 写失败测试**（`shell_integration.rs` 尾部；session 注入测试加在 `session.rs` 的 `mod tests`）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_zdotdir_writes_wrapper_files_idempotently() {
        let dir = ensure_zdotdir().unwrap();
        let zshrc = std::fs::read_to_string(dir.join(".zshrc")).unwrap();
        assert!(zshrc.contains("133;D"), "带退出码发射钩子");
        assert!(zshrc.contains("7;file://"), "带 cwd 发射钩子");
        let zshenv = std::fs::read_to_string(dir.join(".zshenv")).unwrap();
        assert!(zshenv.contains("DOZER_ORIG_ZDOTDIR"));
        // 幂等：重复调用不报错，内容不变
        let dir2 = ensure_zdotdir().unwrap();
        assert_eq!(dir, dir2);
        assert_eq!(zshrc, std::fs::read_to_string(dir.join(".zshrc")).unwrap());
    }
}
```

`session.rs` 的 `mod tests` 追加：

```rust
    #[tokio::test]
    async fn zsh_session_gets_zdotdir_injected() {
        let s = Session::spawn(SessionSpec {
            name: "t".into(),
            command: "/bin/zsh".into(),
            args: vec!["-c".into(), "echo zd=$ZDOTDIR; sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        })
        .unwrap();
        let mut found = false;
        for _ in 0..100 {
            let text = String::from_utf8_lossy(&s.snapshot().0).into_owned();
            if text.contains("zd=") {
                assert!(text.contains("zdotdir"), "ZDOTDIR 应指向包装目录: {text}");
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        assert!(found);
        let _ = s.kill();
    }

    #[tokio::test]
    async fn non_zsh_session_has_no_zdotdir() {
        let s = Session::spawn(SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "echo zd=[$ZDOTDIR]; sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        })
        .unwrap();
        let mut found = false;
        for _ in 0..100 {
            let text = String::from_utf8_lossy(&s.snapshot().0).into_owned();
            if text.contains("zd=") {
                assert!(text.contains("zd=[]"), "非 zsh 不注入: {text}");
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        assert!(found);
        let _ = s.kill();
    }
```

- [x] **Step 3: 跑测试确认失败**

Run: `cargo test -p dozerd`
Expected: 编译错误（`shell_integration` 模块不存在）；zsh 注入测试失败。

- [x] **Step 4: 实现**

`crates/dozerd/src/shell_integration.rs`：

```rust
//! zsh shell 集成：把 OSC 7/133 发射钩子经 ZDOTDIR 包装注入交互式
//! zsh 会话（kitty/ghostty 同款姿势；spec P1e D2）。

use anyhow::{Context, Result};
use std::path::PathBuf;

const ZSHENV: &str = include_str!("../assets/zdotdir/.zshenv");
const ZSHRC: &str = include_str!("../assets/zdotdir/.zshrc");

/// 把包装 zdotdir 落盘到 state 目录（幂等，内容变更时覆写）。
pub fn ensure_zdotdir() -> Result<PathBuf> {
    let dir = dozer_core::paths::state_dir().join("zdotdir");
    std::fs::create_dir_all(&dir).context("创建 zdotdir")?;
    for (name, content) in [(".zshenv", ZSHENV), (".zshrc", ZSHRC)] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
            std::fs::write(&path, content).with_context(|| format!("写 {name}"))?;
        }
    }
    Ok(dir)
}

/// 会话是否应注入 shell 集成：command 是 zsh 且未被 DOZER_SHELL_INTEGRATION=0 关闭。
pub fn should_inject(command: &str) -> bool {
    let is_zsh = std::path::Path::new(command)
        .file_name()
        .is_some_and(|n| n == "zsh");
    is_zsh && std::env::var("DOZER_SHELL_INTEGRATION").as_deref() != Ok("0")
}
```

模块声明：dozerd 的 mod 清单（lib.rs 或 main.rs，以 Task 2 后的实际布局为准）加 `pub mod shell_integration;`。

`session.rs` 的 `spawn`，`cmd.env("DOZER_SESSION_ID", &id);` 之后加：

```rust
        // zsh 会话注入 OSC 7/133 发射端（spec P1e D2；失败仅降级不阻断 spawn）
        if crate::shell_integration::should_inject(&spec.command) {
            match crate::shell_integration::ensure_zdotdir() {
                Ok(wrapper) => {
                    if let Ok(orig) = std::env::var("ZDOTDIR") {
                        cmd.env("DOZER_ORIG_ZDOTDIR", orig);
                    }
                    cmd.env("DOZER_ZDOTDIR_WRAPPER", &wrapper);
                    cmd.env("ZDOTDIR", &wrapper);
                }
                Err(e) => tracing::warn!("shell 集成落盘失败，跳过注入: {e}"),
            }
        }
```

- [x] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozerd`
Expected: 全绿。

- [x] **Step 6: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): zsh 集成 ZDOTDIR 包装注入——OSC 7/133 发射端(DOZER_SHELL_INTEGRATION=0 逃生门)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: dozer-hook UDS 上报 + install/uninstall

**Files:**
- Modify: `crates/dozer-hook/Cargo.toml`（加 `dozer-core = { path = "../dozer-core" }`）
- Modify: `crates/dozer-hook/src/main.rs`
- Create: `crates/dozer-hook/src/install.rs`
- Delete: `crates/dozer-hook/src/payload.rs`（HookPayload 被 `Request::HookEvent` 取代）

**Interfaces:**
- Consumes: Task 1 `Request::HookEvent`、`dozer_core::protocol::encode_line`、`dozer_core::paths::socket_path()`。
- Produces:
  - CLI：`dozer-hook <EventName>`（stdin 收 Claude Code hook JSON，转发 dozerd，恒退出 0）
  - CLI：`dozer-hook install` / `dozer-hook uninstall`（读写 `~/.claude/settings.json`）
  - `install::run_at(path: &std::path::Path, install: bool) -> i32`（测试入口）
  - 注册的 hook 事件集：`SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Notification, Stop, SessionEnd`

- [x] **Step 1: 写失败测试**（`install.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn install_creates_settings_and_registers_all_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, true), 0);
        let root = read(&path);
        for ev in EVENTS {
            let arr = root["hooks"][ev].as_array().expect(ev);
            assert_eq!(arr.len(), 1, "{ev}");
            let cmd = arr[0]["hooks"][0]["command"].as_str().unwrap();
            assert!(cmd.contains("dozer-hook"), "{cmd}");
            assert!(cmd.ends_with(ev), "{cmd}");
        }
    }

    #[test]
    fn install_is_idempotent_and_preserves_foreign_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(run_at(&path, true), 0);
        assert_eq!(run_at(&path, true), 0);
        let root = read(&path);
        assert_eq!(root["model"], "opus", "无关配置保留");
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "他人 hook 保留 + 自己恰一条");
        assert!(stop.iter().any(|e| e["hooks"][0]["command"] == "other-tool"));
    }

    #[test]
    fn uninstall_removes_only_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(run_at(&path, true), 0);
        assert_eq!(run_at(&path, false), 0);
        let root = read(&path);
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "other-tool");
        assert!(root["hooks"]["UserPromptSubmit"].is_null(), "空键移除");
    }

    #[test]
    fn malformed_settings_refused_without_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(run_at(&path, true), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken", "拒写不破坏");
    }
}
```

（`Cargo.toml` dev-dependencies 加 `tempfile = "3"`。）

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-hook`
Expected: 编译错误（`install` 模块不存在）。

- [x] **Step 3: 实现**

`crates/dozer-hook/src/install.rs`：

```rust
//! Claude Code hooks 注册：幂等增量合并 ~/.claude/settings.json，
//! 只增删自己的条目，绝不动用户其他配置（spec P1e D5）。

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// 注册的 hook 事件集（与 dozerd server::agent_state_for 的映射面一致）。
pub const EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

pub fn settings_path() -> PathBuf {
    if let Ok(p) = std::env::var("DOZER_CLAUDE_SETTINGS") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home).join(".claude").join("settings.json")
}

fn entry_is_dozer(entry: &Value) -> bool {
    entry["hooks"]
        .as_array()
        .map(|hs| {
            hs.iter().any(|h| {
                h["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("dozer-hook"))
            })
        })
        .unwrap_or(false)
}

pub fn run_at(path: &Path, install: bool) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => {
            eprintln!("读 {} 失败: {e}", path.display());
            return 1;
        }
    };
    let mut root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-hook".into());

    let Some(obj) = root.as_object_mut() else {
        eprintln!("{} 顶层不是对象，拒绝写入", path.display());
        return 1;
    };
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        eprintln!("hooks 段不是对象，拒绝写入");
        return 1;
    };

    for ev in EVENTS {
        let arr = hooks.entry(ev).or_insert_with(|| json!([]));
        let Some(arr) = arr.as_array_mut() else { continue };
        arr.retain(|e| !entry_is_dozer(e));
        if install {
            arr.push(json!({
                "hooks": [{ "type": "command", "command": format!("{exe} {ev}") }]
            }));
        }
    }
    // 卸载后为空的事件键移除，不留空数组
    let empties: Vec<String> = hooks
        .iter()
        .filter(|(_, v)| v.as_array().is_some_and(|a| a.is_empty()))
        .map(|(k, _)| k.clone())
        .collect();
    for k in empties {
        hooks.remove(&k);
    }

    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("建目录失败: {e}");
            return 1;
        }
    }
    let out = serde_json::to_string_pretty(&root).expect("json serializes") + "\n";
    if let Err(e) = std::fs::write(path, out) {
        eprintln!("写 {} 失败: {e}", path.display());
        return 1;
    }
    println!(
        "{}: {}",
        if install { "已注册" } else { "已移除" },
        path.display()
    );
    0
}
```

`crates/dozer-hook/src/main.rs` 整体替换：

```rust
mod install;

use dozer_core::protocol::{Request, encode_line};
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("install") => std::process::exit(install::run_at(&install::settings_path(), true)),
        Some("uninstall") => std::process::exit(install::run_at(&install::settings_path(), false)),
        other => forward(other.unwrap_or("unknown")),
    }
}

/// 把一次 hook 调用转发给 dozerd。恒静默、恒成功退出——绝不拖慢 agent
/// （spec P1e 错误处理：dozerd 不在/超时/畸形输入一律吞掉）。
fn forward(event_arg: &str) {
    // 非 Dozer 会话（无注入的会话 id）：与 dozerd 无关，直接退出
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        return;
    };
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let data = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let event = if event_arg == "unknown" {
        data.get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    } else {
        event_arg.to_string()
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let req = Request::HookEvent { session_id, event, ts_ms, data };

    let Ok(mut stream) =
        std::os::unix::net::UnixStream::connect(dozer_core::paths::socket_path())
    else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.write_all(encode_line(&req).as_bytes());
    // 单向语义：不读应答，发完即走
}
```

删除 `crates/dozer-hook/src/payload.rs`；`Cargo.toml` 依赖：`dozer-core = { path = "../dozer-core" }`、`serde_json`（已有则保留），dev 加 `tempfile = "3"`。

- [x] **Step 4: 跑测试确认通过 + 手工冒烟**

Run: `cargo test -p dozer-hook`
Expected: 4 测试全绿。

冒烟（dozerd 未运行时恒静默退出 0）：

```bash
cargo build -p dozer-hook
echo '{}' | DOZER_SESSION_ID=nope ./target/aarch64-apple-darwin/debug/dozer-hook Stop; echo "exit=$?"
echo '{}' | ./target/aarch64-apple-darwin/debug/dozer-hook Stop; echo "exit=$?"
```

Expected: 两次均 `exit=0`，无输出。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-hook Cargo.lock
git commit -m "feat(dozer-hook): UDS 上报 HookEvent + install/uninstall 幂等注册(增量合并不动他人配置)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: 全量回归 + 端到端人工验收 + 落档

**Files:**
- Create: `docs/superpowers/specs/2026-07-18-p1e-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（验收通过后 §3 需求 1 回填）
- Modify: 本计划文件（勾选 checkbox）

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录；P1f（验收闭环）起点状态。

- [x] **Step 1: 全量回归**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
```

Expected: workspace 全绿零警告。

- [x] **Step 2: 用户人工验收（逐项 ✓/✗；验收权在用户，实施方不代签）**

```markdown
# P1e 人工验收清单（用户实机执行）
1. `cargo run -p dozer-app` 新建 tab：`cd` 几层目录 → tab 标题跟随目录名；`false` 回车 → 终端栏出现红字 `exit 1`，再跑一条命令提示消失
2. `dozer-hook install` → `~/.claude/settings.json` 出现 7 个事件的 dozer-hook 条目，原有配置无损；重复执行无重复条目
3. tab 里跑 `claude` 派个活 → tab 胶囊变绿"运行中"→ agent 提问/要权限时变紫"待输入"→ 回合结束变金"回合毕"
4. 关 app 重开 → 胶囊状态恢复（registry 最新态经 SessionInfo 下发）
5. 停掉 dozerd（`pkill dozerd`）后在 Dozer 外的终端跑 claude → 无报错无卡顿（hook 静默）
6. `DOZER_SHELL_INTEGRATION=0` 起 dozerd → 新 tab 标题不再跟随 cwd（逃生门生效），去掉后恢复
7. `dozer-hook uninstall` → settings.json 里 dozer 条目干净移除，他人配置无损
```

- [x] **Step 3: 结果落档 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-18-p1e-acceptance.md`（格式沿用 P1c/P1d：逐项结果表 + 反馈修复流水）；全部通过后规格 §3 需求 1 的 P1c 达成标注后追加"P1e 达成（<日期>，shell 集成 OSC 7/133 + hook 事件通路 + agent 状态胶囊；交付横幅/验收闭环随 P1f），验收记录见 specs/2026-07-18-p1e-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1e agent 集成人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **Spec 覆盖**：目标 1（OSC 7/133 + zsh 注入）= T4+T5；目标 2（hook 通路）= T1+T2+T6；目标 3（状态胶囊）= T2 映射 + T3 UI；目标 4（install/uninstall）= T6。裁决 D1=T4（含观察式偏差注记）、D2=T5、D3=T1/T2（AgentEvent 经 attach 流按会话下发——比 spec"广播所有客户端"更窄且足够，晚 attach 由 SessionInfo.agent_state 兜底）、D4=T2 注入 + T6 读取、D5=T6、D6=T2 映射表。错误处理五条分别落 T6（静默退出）、T2（未知会话 Ok+日志）、T4（超长放弃）、T6（拒写畸形 settings）、T5（非 zsh 静默降级）。
2. **占位符扫描**：无 TBD；T2"若 dozerd 模块未 pub 导出则建 lib.rs"是环境适配指引，附带了明确动作。
3. **类型一致性**：`AgentState`（Copy）、`Request::HookEvent{session_id,event,ts_ms,data}`、`Reply::AgentEvent{session_id,state,event,ts_ms}`、`SessionEvent::Agent{state,event,ts_ms}`、`TermEvent::Agent(AgentState)`、`Message::AgentStateChanged(usize,AgentState)`、`agent_state_for(&str)->Option<AgentState>`、`OscEvent::{Cwd(PathBuf),CmdStart,CmdExit(i32)}`、`ensure_zdotdir()->Result<PathBuf>`、`run_at(&Path,bool)->i32` 各任务交叉引用一致；T6 注册的 7 事件集与 T2 映射表逐一对应。
