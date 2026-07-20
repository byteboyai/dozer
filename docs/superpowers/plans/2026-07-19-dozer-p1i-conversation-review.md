# Dozer P1i 对话可审阅性（锚点+折叠）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从 hook data 拿到当前 claude 会话的 transcript 路径，解析成结构化对话（人类锚点 + AI 回合），在左二"会话审阅" tab 呈现：人类发言醒目、AI 回合默认显正文/折叠 thinking+工具，打开时解析、回合结束刷新。

**Architecture:** dozerd 从 `HookEvent.data.transcript_path` 提取存 Session，经 `AgentEvent`+`SessionInfo` 带给 GUI（协议加两个 `Option<String>`，向后兼容）；transcript 适配器是 dozer-app 侧纯函数（`parse_transcript(jsonl)->Vec<ReviewEntry>`）；左二 `TabKind::Review`（iced 直绘，同 Acceptance——激活时 webview 隐藏）；解析在 GUI 侧 spawn_blocking，dozerd 不碰 transcript（只传路径）。

**Tech Stack:** Rust、serde_json（transcript 逐行解析）、iced 0.14、tokio、tempfile（测试）。

**Spec:** `docs/superpowers/specs/2026-07-19-dozer-p1i-conversation-review-design.md`

## Global Constraints

- dozerd 只传 transcript 路径，不解析（哑管道）；解析在 GUI 侧 spawn_blocking。
- 协议向后兼容：新字段 `#[serde(default)]`，旧 JSON 可解。
- 主题：人类锚点 `theme::CREAM` 醒目；AI 正文 `theme::BODY`；折叠"过程"行/thinking `theme::DIM`；工具行 `theme::CYAN`；错误 `theme::RED`。
- 终端 pane 仍是裸对话事实真相；本视图并行，不解析裸 PTY。
- 首片只做 Claude Code transcript + 锚点/折叠（a+b）；提问置顶(c)/产物联动(d)/多 agent 后续。
- 测试全 headless；GUI 行为归末任务人工验收。
- commit 中文、结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；每 Task 收尾 `cargo clippy --all-targets && cargo fmt` 零警告。

---

### Task 1: 协议——transcript_path 字段

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces:
  - `SessionInfo.transcript_path: Option<String>`（`#[serde(default)]`）
  - `Reply::AgentEvent.transcript_path: Option<String>`

- [x] **Step 1: 写失败测试**（`protocol.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn agent_event_carries_transcript_path() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s".into(),
            state: AgentState::Running,
            event: "UserPromptSubmit".into(),
            ts_ms: 1,
            transcript_path: Some("/t/x.jsonl".into()),
        });
        assert!(line.contains("/t/x.jsonl"));
        match decode_line::<Reply>(line.trim()).unwrap() {
            Reply::AgentEvent { transcript_path, .. } => {
                assert_eq!(transcript_path.as_deref(), Some("/t/x.jsonl"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn old_session_info_without_transcript_path_decodes_none() {
        let old = r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.transcript_path, None);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-core transcript`
Expected: 编译错误（字段不存在）。

- [x] **Step 3: 实现**

`SessionInfo` 末尾加：

```rust
    /// 当前会话 agent 的 transcript 文件路径（Claude Code JSONL；hook 携带）。
    #[serde(default)]
    pub transcript_path: Option<String>,
```

`Reply::AgentEvent` 加字段：

```rust
    AgentEvent {
        session_id: String,
        state: AgentState,
        event: String,
        ts_ms: u64,
        /// 该会话最新已知的 transcript 路径（hook data 携带；无则 None）。
        transcript_path: Option<String>,
    },
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-core && cargo build --workspace`
Expected: dozer-core 全绿。dozerd/dozer-client 若因构造 `SessionInfo`/`AgentEvent` 缺字段报错，是预期——Task 2/4 补齐；本步只需 dozer-core 绿（`cargo test -p dozer-core`）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-core
git commit -m "feat(协议): SessionInfo/AgentEvent 加 transcript_path——P1i 会话审阅数据通路

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: dozerd——存 transcript_path + hook 提取 + 广播

**Files:**
- Modify: `crates/dozerd/src/session.rs`
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: Task 1 字段。
- Produces:
  - `Session::set_transcript_path(&self, path: &str)`
  - `Session` 的 `SessionInfo`/`SessionEvent::Agent` 带 transcript_path
  - HookEvent 处理：从 `data["transcript_path"]` 提取并存

- [x] **Step 1: 写失败测试**（`session.rs` 的 `mod tests` 追加）

```rust
    #[tokio::test]
    async fn transcript_path_stored_and_in_info_and_broadcast() {
        let s = Session::spawn(SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        })
        .unwrap();
        assert_eq!(s.info().transcript_path, None);
        let mut rx = s.subscribe();
        s.set_transcript_path("/t/conv.jsonl");
        s.set_agent_state(AgentState::Running, "UserPromptSubmit", 1);
        assert_eq!(s.info().transcript_path.as_deref(), Some("/t/conv.jsonl"));
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                SessionEvent::Agent { transcript_path, .. } => {
                    assert_eq!(transcript_path.as_deref(), Some("/t/conv.jsonl"));
                    break;
                }
                _ => continue,
            }
        }
        let _ = s.kill();
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd transcript_path_stored`
Expected: 编译错误（`set_transcript_path`/字段不存在）。

- [x] **Step 3: 实现**

`session.rs`：`SessionEvent::Agent` 加字段：

```rust
    Agent {
        state: AgentState,
        event: String,
        ts_ms: u64,
        transcript_path: Option<String>,
    },
```

`Session` 结构体加 `transcript_path: Mutex<Option<String>>,`（`spawn` 的 `Ok(Self{...})` 初始化 `transcript_path: Mutex::new(None),`）。

`info()` 的 `SessionInfo{...}` 加：

```rust
            transcript_path: self.transcript_path.lock().expect("tp lock").clone(),
```

加方法：

```rust
    /// 记下会话的 transcript 路径（hook data 携带；覆盖旧值）。
    pub fn set_transcript_path(&self, path: &str) {
        *self.transcript_path.lock().expect("tp lock") = Some(path.to_string());
    }
```

`set_agent_state` 的广播带上当前路径：

```rust
    pub fn set_agent_state(&self, state: AgentState, event: &str, ts_ms: u64) {
        *self.agent_state.lock().expect("agent_state lock") = state;
        let transcript_path = self.transcript_path.lock().expect("tp lock").clone();
        let _ = self.tx.send(SessionEvent::Agent {
            state,
            event: event.to_string(),
            ts_ms,
            transcript_path,
        });
    }
```

`server.rs`：`SessionEvent::Agent` 转 `Reply::AgentEvent` 处加字段（找到 attach 转发里的 `SessionEvent::Agent { state, event, ts_ms }` 分支，补 `transcript_path`）：

```rust
                    Ok(SessionEvent::Agent { state, event, ts_ms, transcript_path }) => {
                        let reply = Reply::AgentEvent { session_id: sid, state, event, ts_ms, transcript_path };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                    }
```

HookEvent 处理：`data: _` 改 `data`，提取路径（`set_agent_state` 调用之前）：

```rust
                        Request::HookEvent { session_id, event, ts_ms, data } => {
                            match registry.get(&session_id) {
                                None => {
                                    tracing::debug!(%session_id, %event, "hook 事件的会话不存在，丢弃");
                                }
                                Some(s) => {
                                    if let Some(tp) = data.get("transcript_path").and_then(|v| v.as_str()) {
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

（`session.rs` 里既有测试构造 `SessionEvent::Agent {...}` 的地方补 `transcript_path: None,`——`set_agent_state_updates_info_and_broadcasts` 的 match 臂已解构 `..` 则无需改；若显式列字段则补。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd`
Expected: 全绿（含新测）。

- [x] **Step 5: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): 存 transcript_path(hook data 提取) + info/AgentEvent 广播携带

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: transcript.rs——JSONL 适配器

**Files:**
- Create: `crates/dozer-app/src/transcript.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod project;` 后加 `mod transcript;`）

**Interfaces:**
- Produces:
  - `ReviewEntry`（`Human{text:String}` / `AiTurn{text:String, tools:Vec<String>, thinking:bool}`；Debug/Clone/PartialEq）
  - `parse_transcript(jsonl: &str) -> Vec<ReviewEntry>`

- [x] **Step 1: 写失败测试**（`transcript.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
        let entries = parse_transcript(jsonl);
        assert_eq!(entries.len(), 2, "1 人类 + 1 AI 回合;工具结果/噪音/坏行跳过");
        assert_eq!(entries[0], ReviewEntry::Human { text: "改一下 README".into() });
        match &entries[1] {
            ReviewEntry::AiTurn { text, tools, thinking } => {
                assert_eq!(text, "好的，我来改。");
                assert!(*thinking);
                assert_eq!(tools, &vec!["Edit README.md".to_string(), "Bash cargo test --all".to_string()]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_and_all_noise_yield_nothing() {
        assert!(parse_transcript("").is_empty());
        assert!(parse_transcript("{\"type\":\"mode\"}\nbad\n").is_empty());
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app parses_human`
Expected: 编译错误（模块不存在）。

- [x] **Step 3: 实现**

`crates/dozer-app/src/transcript.rs`：

```rust
//! Claude Code transcript(JSONL)适配器（P1i）：逐行解析成结构化审阅条目。
//! 纯函数,不碰 iced/IO。规则见 spec P1i D2。

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewEntry {
    /// 人类发言（导航锚点）。
    Human { text: String },
    /// AI 一个回合：正文 + 工具一行摘要 + 是否含 thinking。
    AiTurn {
        text: String,
        tools: Vec<String>,
        thinking: bool,
    },
}

/// 一个 tool_use 块 → 一行摘要 `<name> <主参>`（spec P1i D2）。
fn tool_summary(name: &str, input: &Value) -> String {
    let arg = input
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string())
        })
        .or_else(|| {
            input
                .get("command")
                .and_then(|v| v.as_str())
                .map(|c| c.chars().take(40).collect())
        })
        .or_else(|| {
            input
                .get("pattern")
                .or_else(|| input.get("path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    match arg {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    }
}

pub fn parse_transcript(jsonl: &str) -> Vec<ReviewEntry> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                // content 是字符串 → 人类发言;是 list(工具结果) → 跳过
                if let Some(text) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    out.push(ReviewEntry::Human {
                        text: text.to_string(),
                    });
                }
            }
            Some("assistant") => {
                let Some(blocks) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                let mut text = String::new();
                let mut tools = Vec::new();
                let mut thinking = false;
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            tools.push(tool_summary(name, &input));
                        }
                        Some("thinking") => thinking = true,
                        _ => {}
                    }
                }
                out.push(ReviewEntry::AiTurn {
                    text,
                    tools,
                    thinking,
                });
            }
            _ => {}
        }
    }
    out
}
```

`main.rs` 的 `mod project;` 后加 `mod transcript;`。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app parses_human empty_and_all`
Expected: 2 测试通过。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/transcript.rs crates/dozer-app/src/main.rs
git commit -m "feat(审阅): transcript.rs JSONL 适配器——人类锚点/AI 回合(正文+工具摘要+thinking)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: client + workspace——transcript_path 透传到 tab

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 1/2 字段。
- Produces:
  - `TermEvent::Agent { state: AgentState, transcript_path: Option<String> }`
  - `Message::AgentStateChanged(usize, AgentState, Option<String>)`
  - `SessionTab.transcript_path: Option<String>`（bootstrap 从 SessionInfo，AgentEvent 更新）

- [x] **Step 1: 写失败测试**（`workspace.rs` 的 `mod tests` 追加——纯函数级：审阅入口可见性）

```rust
    #[test]
    fn review_available_needs_transcript() {
        assert!(review_available(Some("/t/x.jsonl")));
        assert!(!review_available(None));
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app review_available`
Expected: 编译错误（`review_available` 未定义）。

- [x] **Step 3: 实现**

`dozer-client/src/lib.rs`：

`TermEvent::Agent(AgentState)` 改为：

```rust
    /// 本会话 agent 状态变更（hook 事件驱动，dozerd 广播）。
    Agent {
        state: AgentState,
        transcript_path: Option<String>,
    },
```

attach 解码 `Ok(Reply::AgentEvent { state, .. }) => TermEvent::Agent(state),` 改为：

```rust
                                    Ok(Reply::AgentEvent { state, transcript_path, .. }) => {
                                        TermEvent::Agent { state, transcript_path }
                                    }
```

`dozer-app/src/workspace.rs`：

1. `Message::AgentStateChanged(usize, AgentState)` 改为 `AgentStateChanged(usize, AgentState, Option<String>)`。

2. `SessionTab` 加字段 `pub transcript_path: Option<String>,`（`bootstrap`/`on_tab_attached` 两处构造初始化 `transcript_path: info.transcript_path.clone(),`——注意 `info` move 顺序，在 move 前 clone）。

3. `forward_events` 的 `TermEvent::Agent(state) => Message::AgentStateChanged(tab_id, state),` 改为：

```rust
            TermEvent::Agent { state, transcript_path } => {
                Message::AgentStateChanged(tab_id, state, transcript_path)
            }
```

4. `Message::AgentStateChanged` handler：签名多一个参数，末尾更新 tab 的 transcript_path（在现有 TurnEnded 逻辑之外）。找到 `Message::AgentStateChanged(tab_id, state) => {` 改为 `(tab_id, state, transcript_path) => {`，在 `if let Some(tab) = self.tab_by_id_mut(tab_id) {` 块内、`tab.agent_state = state;` 之后加：

```rust
                    if let Some(tp) = transcript_path {
                        tab.transcript_path = Some(tp);
                    }
```

5. 纯函数（文件级）：

```rust
/// 会话是否可审阅（有 transcript）——决定终端 tab 是否显示"审阅"入口。
fn review_available(transcript_path: Option<&str>) -> bool {
    transcript_path.is_some()
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-client && cargo test -p dozer-app review_available`
Expected: 全绿。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-client crates/dozer-app/src/workspace.rs
git commit -m "feat(审阅): transcript_path 经 TermEvent::Agent 透传到终端 tab + 审阅入口判定

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: 左二会话审阅 tab——TabKind::Review + 渲染 + 入口 + 刷新

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`（`TabKind::Review` + `open_review`）
- Modify: `crates/dozer-app/src/workspace.rs`（ReviewView + 消息 + 渲染 + 审阅按钮 + 刷新）

**Interfaces:**
- Consumes: Task 3 `ReviewEntry`/`parse_transcript`；Task 4 `SessionTab.transcript_path`、`review_available`。
- Produces:
  - `TabKind::Review`；`PreviewPane::{open_review()->usize, review_active()->bool}`
  - `Message::{ReviewOpen(usize), ReviewLoaded(usize, Result<Vec<ReviewEntry>, String>), ReviewToggle(usize)}`
  - `ai_turn_summary(tools_len: usize, thinking: bool) -> String`（纯函数）

- [x] **Step 1: 写失败测试**

`preview.rs` tests 追加：

```rust
    #[test]
    fn review_tab_no_webview_and_reuse() {
        let mut p = PreviewPane::default();
        p.open_url("http://localhost:3000".into());
        let id = p.open_review();
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, "审阅 tab 不产 webview");
        assert!(!specs[0].visible, "审阅 tab 激活时其余隐藏");
        assert_eq!(p.open_review(), id, "复用同一 tab");
        assert_eq!(p.tabs().len(), 2);
    }
```

`workspace.rs` tests 追加：

```rust
    #[test]
    fn ai_turn_summary_text() {
        assert_eq!(ai_turn_summary(0, false), "过程:无");
        assert_eq!(ai_turn_summary(2, false), "过程:2 工具");
        assert_eq!(ai_turn_summary(2, true), "过程:思考 + 2 工具");
        assert_eq!(ai_turn_summary(0, true), "过程:思考");
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app review_tab_no_webview ai_turn_summary`
Expected: 编译错误（未定义）。

- [x] **Step 3: 实现**

`preview.rs`：`TabKind` 加 `Review`（同 Acceptance 处理）：

```rust
    /// 会话审阅 tab（P1i）:不产 webview,内容由 iced 直绘。
    Review,
```

`open_review`/`review_active`（仿 `open_acceptance`/`acceptance_active`）：

```rust
    pub fn open_review(&mut self) -> usize {
        if let Some((idx, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == TabKind::Review)
        {
            let id = tab.id;
            self.active = idx;
            return id;
        }
        self.push_tab(TabKind::Review, "审阅".to_string())
    }

    pub fn review_active(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.kind == TabKind::Review)
    }
```

`desired_webviews` 的 `TabKind::Acceptance => return None,` 之后加 `TabKind::Review => return None,`；`acceptance_active()` 参与的 `visible` 计算改为"验收或审阅任一激活则隐藏其它 webview"——把 `let acceptance_active = self.acceptance_active();` 后并用 `let overlay_active = acceptance_active || self.review_active();`，`visible: idx == self.active && !overlay_active`。

`workspace.rs`：

1. `use crate::transcript::{self, ReviewEntry};`

2. `ReviewView` 状态 + 字段（Workspace 加 `review: Option<ReviewView>,`，两处构造 `review: None,`）：

```rust
/// 会话审阅 tab 的内容（P1i）。
pub struct ReviewView {
    pub source_tab_id: usize,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 展开了正文过程区的 AI 回合下标（entries 中的位置）。
    pub expanded: std::collections::HashSet<usize>,
}
```

3. `Message` 追加：

```rust
    /// 会话审阅:点终端 tab"审阅"（tab_id 为来源会话）。
    ReviewOpen(usize),
    /// 会话审阅:解析完成（来源 tab_id, 条目 / 错误文案）。
    ReviewLoaded(usize, Result<Vec<ReviewEntry>, String>),
    /// 会话审阅:展开/收起第 n 个 AI 回合的过程区。
    ReviewToggle(usize),
```

4. `update` 追加分支：

```rust
            Message::ReviewOpen(tab_id) => {
                let path = self
                    .tab_by_id_mut(tab_id)
                    .and_then(|t| t.transcript_path.clone());
                let Some(path) = path else { return };
                self.review = Some(ReviewView {
                    source_tab_id: tab_id,
                    entries: Vec::new(),
                    error: None,
                    expanded: std::collections::HashSet::new(),
                });
                self.preview.open_review();
                self.spawn_review_load(tab_id, path);
            }
            Message::ReviewLoaded(tab_id, result) => {
                // 仅当审阅视图仍属该会话时更新
                if let Some(rv) = &mut self.review {
                    if rv.source_tab_id == tab_id {
                        match result {
                            Ok(entries) => {
                                rv.entries = entries;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                }
            }
            Message::ReviewToggle(i) => {
                if let Some(rv) = &mut self.review {
                    if !rv.expanded.remove(&i) {
                        rv.expanded.insert(i);
                    }
                }
            }
```

5. 刷新方法（`impl Workspace`）+ TurnEnded 接线：

```rust
    /// 异步读 transcript + 解析 → ReviewLoaded（GUI 侧 spawn_blocking）。
    fn spawn_review_load(&self, tab_id: usize, path: String) {
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                std::fs::read_to_string(&path)
                    .map(|s| transcript::parse_transcript(&s))
                    .map_err(|e| format!("无法读取会话记录: {e}"))
            })
            .await
            .unwrap_or_else(|e| Err(format!("解析任务失败: {e}")));
            let _ = proxy.send_event(Message::ReviewLoaded(tab_id, result));
        });
    }
```

`AgentStateChanged` 的 TurnEnded 分支里（git 刷新旁）追加：审阅 tab 开着且属该会话则重解析。在 `if state == AgentState::TurnEnded {` 块内、git 检测 spawn 之后加：

```rust
                        // 审阅 tab 若开着且属本会话,回合结束重解析（P1i）。
                        if let Some(rv) = &self.review {
                            if rv.source_tab_id == tab_id
                                && let Some(tab) = self.tab_by_id(tab_id)
                                && let Some(path) = tab.transcript_path.clone()
                            {
                                self.spawn_review_load(tab_id, path);
                            }
                        }
```

（若无 `tab_by_id`（不可变版），用 `self.tabs.iter().find(|t| t.tab_id == tab_id)`。注意借用：`self.review` 只读 + `self.spawn_review_load(&self)` 只读，可共存；实施以借用检查通过为准。）

6. 渲染纯函数 + 内容 + 审阅按钮。`ai_turn_summary`：

```rust
/// AI 回合折叠行文案（P1i）：过程 = thinking + N 工具。
fn ai_turn_summary(tools_len: usize, thinking: bool) -> String {
    match (thinking, tools_len) {
        (false, 0) => "过程:无".into(),
        (true, 0) => "过程:思考".into(),
        (false, n) => format!("过程:{n} 工具"),
        (true, n) => format!("过程:思考 + {n} 工具"),
    }
}
```

`preview_pane` 内容渲染：仿 `acceptance_content`,加 `review_content`。在 `preview_pane` 里 `acceptance_active` 分支旁加 `review_active` 分支调 `review_content(content, ws)`。`review_content`：

```rust
fn review_content<'a>(
    mut content: iced_widget::Column<'a, Message, iced_widget::Theme, iced_widget::Renderer>,
    ws: &'a Workspace,
) -> iced_widget::Column<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(rv) = &ws.review else {
        return content;
    };
    if let Some(err) = &rv.error {
        return content.push(text(format!("⚠ {err}")).size(13).color(theme::RED));
    }
    if rv.entries.is_empty() {
        return content.push(text("暂无对话").size(13).color(theme::DIM));
    }
    for (i, e) in rv.entries.iter().enumerate() {
        match e {
            ReviewEntry::Human { text: t } => {
                content = content.push(text(format!("▎{t}")).size(14).color(theme::CREAM));
            }
            ReviewEntry::AiTurn {
                text: body,
                tools,
                thinking,
            } => {
                if !body.is_empty() {
                    content = content.push(text(body.clone()).size(13).color(theme::BODY));
                }
                let expanded = rv.expanded.contains(&i);
                let glyph = if expanded { "▾ " } else { "▸ " };
                content = content.push(
                    button(
                        text(format!("{glyph}{}", ai_turn_summary(tools.len(), *thinking)))
                            .size(12)
                            .color(theme::DIM),
                    )
                    .on_press(Message::ReviewToggle(i))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::DIM,
                        ..button::Style::default()
                    }),
                );
                if expanded {
                    if *thinking {
                        content = content
                            .push(text("  · 思考(略)").size(11).color(theme::DIM));
                    }
                    for tool in tools {
                        content = content
                            .push(text(format!("  · {tool}")).size(12).color(theme::CYAN));
                    }
                }
            }
        }
    }
    content
}
```

`preview_pane` 的分支接线（`acceptance_active()` 判断旁）：

```rust
    if ws.preview.review_active() {
        content = review_content(content, ws);
    } else if ws.preview.acceptance_active() {
        content = acceptance_content(content, ws);
    } else if ws.preview.tabs().is_empty() {
        // ……原空态……
    }
```

终端 tab 栏"审阅"按钮：在 `tab_bar`（或终端 pane 头部）当前激活 tab 有 transcript 时加。最小接法——在 `terminal_pane` 的 `tab_bar(ws)` 之后、内容之前，若当前 tab `review_available` 则 push 一个"审阅"按钮：

```rust
    if let Some(tab) = ws.tabs.get(ws.active)
        && review_available(tab.transcript_path.as_deref())
    {
        content = content.push(
            button(text("审阅").size(12).color(theme::CYAN))
                .on_press(Message::ReviewOpen(tab.tab_id))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::CYAN,
                    border: Border { color: theme::BORDER, width: 1.0, radius: 2.0.into() },
                    ..button::Style::default()
                }),
        );
    }
```

（放在 `terminal_pane` 里 `content` 已建、`active_tab_view` 之前。）

- [x] **Step 4: 跑测试确认通过 + 冒烟**

```bash
cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 测试全绿、零警告；app 存活 8 秒无 panic。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(审阅): 左二会话审阅 tab——人类锚点 + AI 回合折叠(正文/过程) + 终端审阅入口 + 回合结束刷新

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: 全量回归 + 端到端人工验收 + 落档

**Files:**
- Create: `docs/superpowers/specs/2026-07-19-p1i-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（验收通过后 §3 需求 6 回填）
- Modify: 本计划文件（勾选 checkbox）

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录；下一阶段起点。

- [x] **Step 1: 全量回归 + 冒烟**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 全绿零警告；app 存活 8 秒。**验收前 `pkill dozerd` 重启新 daemon;需 `dozer-hook install` 且新起 claude 会话(transcript_path 靠 hook 携带)。**

- [ ] **Step 2: 用户人工验收（逐项 ✓/✗，验收权在用户）**

```markdown
# P1i 人工验收清单（用户实机执行）
1. Dozer 里新起 claude 会话聊一句 → 终端 tab 栏出现"审阅"入口（shell tab 无）
2. 点"审阅" → 左二会话审阅 tab:人类那句话成醒目锚点行(▎奶油色)
3. AI 回合默认只见正文;点"▸ 过程" → 展开见 思考(略) + 工具行(如 Edit xxx / Bash xxx)
4. 让 claude 再答一轮(含改文件)→ 回合结束后审阅 tab 自动追加新条目
5. 关审阅 tab / 切到别的预览 tab → 正常,webview 不抢层
6. 没跑 claude 的纯 shell tab → 无"审阅"入口
7. 关 app 重开 → 恢复的会话若 transcript 仍在,"审阅"入口仍在(SessionInfo 携带)
```

- [ ] **Step 3: 结果落档 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-19-p1i-acceptance.md`（沿用格式：逐项结果表 + 反馈修复流水）；全部通过后规格 §3 需求 6 追加"P1i 达成（<日期>，对话可审阅性首片：transcript 适配器 + 左二会话审阅 tab 人类锚点 + AI 回合折叠；提问置顶/产物联动/多 agent 随后续），验收记录见 specs/2026-07-19-p1i-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1i 对话可审阅性人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **Spec 覆盖**：D1（transcript_path 经 hook data → dozerd → AgentEvent/SessionInfo）=T1 协议 + T2 dozerd 提取/广播 + T4 client/tab 透传；D2（适配器纯函数 + 工具摘要规则）=T3；D3（Review tab iced 直绘 + 锚点/折叠）=T5；D4（GUI spawn_blocking，dozerd 不解析）=T5 `spawn_review_load`；D5（终端 tab 审阅入口）=T4 `review_available` + T5 按钮；D6（打开 + 回合结束刷新）=T5 ReviewOpen + TurnEnded 接线。错误处理五条：无路径无入口（T4/T5）、读失败红字（T5 ReviewLoaded Err）、坏行跳过（T3）、工具结果 user turn 不成锚点（T3 content 非字符串跳过）、空 transcript 显"暂无对话"（T5）。
2. **占位符扫描**：无 TBD；T5 借用注记（review 只读 + spawn_review_load 只读可共存）给了明确方向与回退（find 替代 tab_by_id）。
3. **类型一致性**：`SessionInfo.transcript_path`/`AgentEvent.transcript_path`/`SessionEvent::Agent.transcript_path`/`TermEvent::Agent{state,transcript_path}`/`Message::AgentStateChanged(usize,AgentState,Option<String>)`/`SessionTab.transcript_path`、`ReviewEntry::{Human,AiTurn}`/`parse_transcript`、`TabKind::Review`/`open_review`/`review_active`、`Message::{ReviewOpen,ReviewLoaded,ReviewToggle}`、`ai_turn_summary(usize,bool)`、`review_available(Option<&str>)` 在 T1-T5 交叉一致。
