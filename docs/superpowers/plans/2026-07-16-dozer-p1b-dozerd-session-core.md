# Dozer P1b：dozerd 会话存活内核实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** dozerd 成为真正的 session daemon：持有 PTY 与滚屏缓冲，客户端经 UDS + JSON Lines 协议创建/附着/写入/杀死会话，**客户端断连后会话与滚屏完整存活**（一期冻结需求 4 的本体）。

**Architecture:** dozerd 改为 lib + bin 结构：`ring`（字节环形缓冲）→ `session`（portable-pty 会话 + 读线程泵 + broadcast 事件）→ `registry`（会话表）→ `server`（UDS 接入 + 协议分发）逐层向上，每层独立测试；协议类型放 `dozer-core::protocol` 供 P1c 的 GUI 客户端复用。会话元数据一期驻内存（daemon 自身重启后的恢复依赖 agent resume id，归 P1f）。

**Tech Stack:** tokio 1（net/io-util/sync/signal）· portable-pty 0.8 · uuid v4 · base64 0.22 · serde/serde_json

**规格来源:** `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md` §3-需求4、§4-终端数据面/IPC、§5。**P1a 移交约束**：产物在 `target/aarch64-apple-darwin/debug/`；`dozer_core::paths::socket_path()` 已存在。

## Global Constraints

- Rust edition 2024；workspace resolver "3"；核心不引入 Node/Python 运行时。
- IPC = **UDS + JSON Lines**（每帧一行 JSON，`\n` 结尾）；字节数据在 JSON 内一律 base64（字段名以 `_b64` 结尾）。
- 滚屏环形缓冲容量常量 `SCROLLBACK_CAP: usize = 1 << 20`（1 MiB/会话，一期定值）。
- 会话存活语义：客户端断连/崩溃**不得**终止 PTY 或丢失缓冲；只有显式 `kill` 或子进程自然退出才结束会话。
- 测试不得依赖交互 TTY 行为之外的东西：统一用 `/bin/sh -c '…'` 做子进程；socket 用 `std::env::temp_dir()` 下唯一路径，测试结束删除。
- commit message 中文，末尾空行后 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。
- 每任务结束：`cargo test -p <crate>` 全绿 + `cargo clippy --all-targets` 无警告 + commit。
- API 漂移处理：portable-pty 以 docs.rs/portable-pty/0.8 为准对齐，验收标准不变。

---

### Task 1: 协议类型与 JSON Lines 编解码（dozer-core::protocol）

**Files:**
- Modify: `Cargo.toml`（根：workspace.dependencies 增 `uuid = { version = "1", features = ["v4"] }`、`base64 = "0.22"`、`portable-pty = "0.8"`）
- Modify: `crates/dozer-core/Cargo.toml`（增 serde_json 依赖）
- Create: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozer-core/src/lib.rs`（增 `pub mod protocol;`）

**Interfaces:**
- Consumes: workspace serde/serde_json。
- Produces（P1c GUI 客户端与本计划后续任务的精确契约）:
  - `protocol::Request`（枚举，`serde(tag="type", rename_all="snake_case")`）：`ListSessions` / `CreateSession{name,command,args,cwd,cols,rows}` / `Attach{session_id,from_offset}` / `Write{session_id,data_b64}` / `Resize{session_id,cols,rows}` / `Kill{session_id}`
  - `protocol::Reply`（同 tag 风格）：`Sessions{sessions}` / `Created{session}` / `Attached{session_id,snapshot_b64,next_offset}` / `Output{session_id,data_b64,offset}` / `Exited{session_id,code}` / `Ok` / `Error{message}`
  - `protocol::SessionInfo{id:String,name:String,command:String,cwd:String,alive:bool,created_ms:u64}`
  - `protocol::{encode_line<T:Serialize>(&T)->String, decode_line<T:DeserializeOwned>(&str)->anyhow::Result<T>}`

- [ ] **Step 1: 写失败测试**

```rust
// crates/dozer-core/src/protocol.rs 尾部
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_as_single_json_line() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
        };
        let line = encode_line(&req);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn reply_tag_is_snake_case() {
        let line = encode_line(&Reply::Attached {
            session_id: "s1".into(),
            snapshot_b64: "aGk=".into(),
            next_offset: 2,
        });
        assert!(line.contains(r#""type":"attached""#));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_line::<Request>("not json").is_err());
    }
}
```

Run: `cargo test -p dozer-core protocol`
Expected: FAIL —— 类型未定义。

- [ ] **Step 2: 最小实现**

```rust
// crates/dozer-core/src/protocol.rs（测试模块上方）
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub alive: bool,
    pub created_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    ListSessions,
    CreateSession { name: String, command: String, args: Vec<String>, cwd: String, cols: u16, rows: u16 },
    Attach { session_id: String, from_offset: u64 },
    Write { session_id: String, data_b64: String },
    Resize { session_id: String, cols: u16, rows: u16 },
    Kill { session_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Sessions { sessions: Vec<SessionInfo> },
    Created { session: SessionInfo },
    Attached { session_id: String, snapshot_b64: String, next_offset: u64 },
    Output { session_id: String, data_b64: String, offset: u64 },
    Exited { session_id: String, code: Option<i32> },
    Ok,
    Error { message: String },
}

pub fn encode_line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).expect("protocol types always serialize");
    s.push('\n');
    s
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(line)?)
}
```

`crates/dozer-core/Cargo.toml` 的 `[dependencies]` 增：`serde_json.workspace = true`。根 `Cargo.toml` 的 `[workspace.dependencies]` 增三行：

```toml
uuid = { version = "1", features = ["v4"] }
base64 = "0.22"
portable-pty = "0.8"
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p dozer-core`
Expected: PASS（原 3 个 paths 测试 + 新 3 个 protocol 测试）。

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(core): dozerd 协议类型与 JSON Lines 编解码

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: 字节环形缓冲 RingBuffer（dozerd::ring）

**Files:**
- Modify: `crates/dozerd/Cargo.toml`（结构转 lib+bin：无需改 toml，只需加 src/lib.rs）
- Create: `crates/dozerd/src/lib.rs`
- Create: `crates/dozerd/src/ring.rs`

**Interfaces:**
- Consumes: 无。
- Produces:
  - `ring::SCROLLBACK_CAP: usize`（= 1 << 20）
  - `ring::RingBuffer`：`new(cap: usize)` / `push(&mut self, data: &[u8]) -> u64`（返回 push 后的 total_written）/ `total_written(&self) -> u64` / `snapshot(&self) -> (Vec<u8>, u64)`（缓冲现存字节 + next_offset）/ `read_from(&self, offset: u64) -> Option<Vec<u8>>`（offset 仍在窗口内则返回其后的字节；已被挤出则 None）

- [ ] **Step 1: 写失败测试**

```rust
// crates/dozerd/src/ring.rs 尾部
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_accumulates_and_snapshot_returns_all_when_under_cap() {
        let mut r = RingBuffer::new(16);
        assert_eq!(r.push(b"hello"), 5);
        assert_eq!(r.push(b" world"), 11);
        let (data, next) = r.snapshot();
        assert_eq!(data, b"hello world");
        assert_eq!(next, 11);
        assert_eq!(r.total_written(), 11);
    }

    #[test]
    fn eviction_keeps_only_last_cap_bytes() {
        let mut r = RingBuffer::new(8);
        r.push(b"0123456789"); // 10 bytes into cap 8
        let (data, next) = r.snapshot();
        assert_eq!(data, b"23456789");
        assert_eq!(next, 10);
    }

    #[test]
    fn read_from_returns_tail_or_none_when_evicted() {
        let mut r = RingBuffer::new(8);
        r.push(b"0123456789");
        assert_eq!(r.read_from(6).unwrap(), b"6789");
        assert_eq!(r.read_from(10).unwrap(), b"" as &[u8]);
        assert!(r.read_from(1).is_none()); // offset 1 已被挤出（窗口起点=2）
    }

    #[test]
    fn oversized_push_keeps_last_cap_bytes() {
        let mut r = RingBuffer::new(4);
        r.push(b"abcdefgh");
        let (data, next) = r.snapshot();
        assert_eq!(data, b"efgh");
        assert_eq!(next, 8);
    }
}
```

`crates/dozerd/src/lib.rs`：

```rust
pub mod ring;
```

Run: `cargo test -p dozerd`
Expected: FAIL —— RingBuffer 未定义。

- [ ] **Step 2: 最小实现**

```rust
// crates/dozerd/src/ring.rs（测试模块上方）
use std::collections::VecDeque;

/// 每会话滚屏缓冲容量：1 MiB（一期定值，规格 §4 终端数据面）
pub const SCROLLBACK_CAP: usize = 1 << 20;

/// 追加写、按容量自动逐出头部的字节环。offset 为单调递增的"历史总写入量"坐标系。
pub struct RingBuffer {
    cap: usize,
    buf: VecDeque<u8>,
    total: u64,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self { cap, buf: VecDeque::with_capacity(cap.min(64 * 1024)), total: 0 }
    }

    pub fn push(&mut self, data: &[u8]) -> u64 {
        self.buf.extend(data.iter().copied());
        while self.buf.len() > self.cap {
            self.buf.pop_front();
        }
        self.total += data.len() as u64;
        self.total
    }

    pub fn total_written(&self) -> u64 {
        self.total
    }

    /// 窗口起点的 offset（第一个仍在缓冲内的字节的历史坐标）
    fn window_start(&self) -> u64 {
        self.total - self.buf.len() as u64
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        (self.buf.iter().copied().collect(), self.total)
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        if offset > self.total {
            return None;
        }
        if offset < self.window_start() {
            return None; // 已被逐出，调用方应退回全量 snapshot
        }
        let skip = (offset - self.window_start()) as usize;
        Some(self.buf.iter().skip(skip).copied().collect())
    }
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p dozerd`
Expected: PASS（4 个）。

- [ ] **Step 4: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): 滚屏字节环形缓冲 RingBuffer（offset 坐标系 + 逐出语义）

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: PTY 会话 Session（dozerd::session）

**Files:**
- Modify: `crates/dozerd/Cargo.toml`（增 portable-pty/uuid/dozer-core 依赖）
- Create: `crates/dozerd/src/session.rs`
- Modify: `crates/dozerd/src/lib.rs`（增 `pub mod session;`）

**Interfaces:**
- Consumes: `ring::{RingBuffer, SCROLLBACK_CAP}`；`dozer_core::protocol::SessionInfo`。
- Produces:
  - `session::SessionEvent`：`Output { data: Vec<u8>, offset: u64 }` / `Exited { code: Option<i32> }`（`#[derive(Clone)]`）
  - `session::SessionSpec { name, command, args, cwd, cols, rows }`（字段类型同协议 CreateSession）
  - `session::Session`：`spawn(spec: SessionSpec) -> anyhow::Result<Session>` / `info(&self) -> SessionInfo`（alive 实时）/ `write(&self, data: &[u8]) -> anyhow::Result<()>` / `resize(&self, cols: u16, rows: u16) -> anyhow::Result<()>` / `kill(&self) -> anyhow::Result<()>` / `snapshot(&self) -> (Vec<u8>, u64)` / `read_from(&self, offset: u64) -> Option<Vec<u8>>` / `subscribe(&self) -> tokio::sync::broadcast::Receiver<SessionEvent>`

- [ ] **Step 1: 补依赖**

`crates/dozerd/Cargo.toml` 的 `[dependencies]` 增：

```toml
portable-pty.workspace = true
uuid.workspace = true
serde_json.workspace = true
```

（dozer-core 已在依赖中。）

- [ ] **Step 2: 写失败测试**

```rust
// crates/dozerd/src/session.rs 尾部
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(cmd: &str) -> SessionSpec {
        SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        }
    }

    /// 等待直到缓冲包含期望内容或超时（PTY 输出是异步的）
    async fn wait_contains(s: &Session, needle: &[u8]) -> bool {
        for _ in 0..100 {
            let (data, _) = s.snapshot();
            if data.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        false
    }

    #[tokio::test]
    async fn output_lands_in_buffer_and_broadcast() {
        let s = Session::spawn(spec("printf marker123; sleep 5")).unwrap();
        let mut rx = s.subscribe();
        assert!(wait_contains(&s, b"marker123").await, "buffer should contain output");
        // broadcast 也应收到含 marker 的事件（可能分片，收多次拼接）
        let mut got = Vec::new();
        while let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            if let SessionEvent::Output { data, .. } = ev {
                got.extend(data);
                if got.windows(9).any(|w| w == b"marker123") {
                    break;
                }
            }
        }
        assert!(got.windows(9).any(|w| w == b"marker123"));
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn write_reaches_child_stdin() {
        let s = Session::spawn(spec("cat")).unwrap();
        s.write(b"pingpong\n").unwrap();
        // cat 回显（经 PTY，含回显本身）
        assert!(wait_contains(&s, b"pingpong").await);
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn natural_exit_broadcasts_exited_and_marks_dead() {
        let s = Session::spawn(spec("printf done")).unwrap();
        let mut rx = s.subscribe();
        let mut exited = false;
        for _ in 0..100 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(SessionEvent::Exited { .. })) => {
                    exited = true;
                    break;
                }
                Ok(Ok(_)) => continue,
                _ => continue,
            }
        }
        assert!(exited, "should broadcast Exited");
        assert!(!s.info().alive);
    }
}
```

`lib.rs` 增 `pub mod session;`。

Run: `cargo test -p dozerd session`
Expected: FAIL —— 类型未定义。

- [ ] **Step 3: 实现**

```rust
// crates/dozerd/src/session.rs（测试模块上方）
use crate::ring::{RingBuffer, SCROLLBACK_CAP};
use anyhow::{Context, Result};
use dozer_core::protocol::SessionInfo;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Output { data: Vec<u8>, offset: u64 },
    Exited { code: Option<i32> },
}

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct Session {
    id: String,
    spec: SessionSpec,
    created_ms: u64,
    alive: Arc<AtomicBool>,
    buffer: Arc<Mutex<RingBuffer>>,
    tx: broadcast::Sender<SessionEvent>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis() as u64
}

impl Session {
    pub fn spawn(spec: SessionSpec) -> Result<Self> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 })
            .context("openpty")?;
        let mut cmd = CommandBuilder::new(&spec.command);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        let child = pair.slave.spawn_command(cmd).context("spawn_command")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;

        let alive = Arc::new(AtomicBool::new(true));
        let buffer = Arc::new(Mutex::new(RingBuffer::new(SCROLLBACK_CAP)));
        let (tx, _) = broadcast::channel::<SessionEvent>(1024);

        // 读线程：PTY 是阻塞 IO，用 std::thread 泵到 ring + broadcast
        {
            let alive = alive.clone();
            let buffer = buffer.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let data = chunk[..n].to_vec();
                            let offset = buffer.lock().expect("ring lock").push(&data);
                            let _ = tx.send(SessionEvent::Output { data, offset });
                        }
                    }
                }
                alive.store(false, Ordering::SeqCst);
                let _ = tx.send(SessionEvent::Exited { code: None });
            });
        }

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_ms: now_ms(),
            alive,
            buffer,
            tx,
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            spec,
        })
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            name: self.spec.name.clone(),
            command: self.spec.command.clone(),
            cwd: self.spec.cwd.clone(),
            alive: self.alive.load(Ordering::SeqCst),
            created_ms: self.created_ms,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().expect("writer lock");
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .lock()
            .expect("master lock")
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn kill(&self) -> Result<()> {
        self.child.lock().expect("child lock").kill()?;
        Ok(())
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        self.buffer.lock().expect("ring lock").snapshot()
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        self.buffer.lock().expect("ring lock").read_from(offset)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.tx.subscribe()
    }
}
```

注意：`Exited { code: None }` 一期不取真实退出码（portable-pty 的 wait 会与 kill 竞争锁，简化处理），协议字段保留 `Option<i32>`，P1c 不依赖具体码值。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozerd session`
Expected: PASS（3 个，均带真实子进程）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): PTY 会话——spawn/write/resize/kill + 读线程泵入环形缓冲与广播

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: 会话表 SessionRegistry（dozerd::registry）

**Files:**
- Create: `crates/dozerd/src/registry.rs`
- Modify: `crates/dozerd/src/lib.rs`（增 `pub mod registry;`）

**Interfaces:**
- Consumes: `session::{Session, SessionSpec}`。
- Produces:
  - `registry::SessionRegistry`：`new() -> Self` / `create(&self, spec: SessionSpec) -> anyhow::Result<Arc<Session>>` / `get(&self, id: &str) -> Option<Arc<Session>>` / `list(&self) -> Vec<dozer_core::protocol::SessionInfo>` / `kill(&self, id: &str) -> anyhow::Result<()>`（kill 后仍保留在表中，alive=false——**死会话的滚屏仍可 attach 查看**）

- [ ] **Step 1: 写失败测试**

```rust
// crates/dozerd/src/registry.rs 尾部
#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionSpec;
    use std::time::Duration;

    fn spec(cmd: &str) -> SessionSpec {
        SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        }
    }

    #[tokio::test]
    async fn create_get_list_roundtrip() {
        let reg = SessionRegistry::new();
        let s = reg.create(spec("sleep 5")).unwrap();
        let id = s.id().to_string();
        assert!(reg.get(&id).is_some());
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].id, id);
        reg.kill(&id).unwrap();
    }

    #[tokio::test]
    async fn killed_session_stays_listed_with_scrollback() {
        let reg = SessionRegistry::new();
        let s = reg.create(spec("printf tomb; sleep 5")).unwrap();
        let id = s.id().to_string();
        // 等输出落缓冲
        for _ in 0..100 {
            if s.snapshot().0.windows(4).any(|w| w == b"tomb") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        reg.kill(&id).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let listed = reg.list();
        assert_eq!(listed.len(), 1, "dead session remains listed");
        assert!(!listed[0].alive);
        let (data, _) = reg.get(&id).unwrap().snapshot();
        assert!(data.windows(4).any(|w| w == b"tomb"), "scrollback survives kill");
    }

    #[tokio::test]
    async fn get_unknown_is_none_and_kill_unknown_errs() {
        let reg = SessionRegistry::new();
        assert!(reg.get("nope").is_none());
        assert!(reg.kill("nope").is_err());
    }
}
```

Run: `cargo test -p dozerd registry`
Expected: FAIL。

- [ ] **Step 2: 最小实现**

```rust
// crates/dozerd/src/registry.rs（测试模块上方）
use crate::session::{Session, SessionSpec};
use anyhow::{Result, anyhow};
use dozer_core::protocol::SessionInfo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct SessionRegistry {
    map: Mutex<HashMap<String, Arc<Session>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, spec: SessionSpec) -> Result<Arc<Session>> {
        let s = Arc::new(Session::spawn(spec)?);
        self.map.lock().expect("registry lock").insert(s.id().to_string(), s.clone());
        Ok(s)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.map.lock().expect("registry lock").get(id).cloned()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut v: Vec<SessionInfo> =
            self.map.lock().expect("registry lock").values().map(|s| s.info()).collect();
        v.sort_by_key(|i| i.created_ms);
        v
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        self.get(id).ok_or_else(|| anyhow!("会话不存在: {id}"))?.kill()
    }
}
```

- [ ] **Step 3: 跑测试 + Commit**

Run: `cargo test -p dozerd`
Expected: PASS（ring 4 + session 3 + registry 3）。

```bash
git add crates/dozerd
git commit -m "feat(dozerd): SessionRegistry——死会话保留滚屏可查

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: UDS 服务端与会话存活集成测试（dozerd::server）

**Files:**
- Create: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/lib.rs`（增 `pub mod server;`）
- Modify: `crates/dozerd/Cargo.toml`（增 base64；[dev-dependencies] 无需新增）
- Create: `crates/dozerd/tests/session_survival.rs`

**Interfaces:**
- Consumes: Task 1 协议、Task 3/4 会话与注册表。
- Produces: `server::serve(socket: &std::path::Path, registry: Arc<SessionRegistry>) -> anyhow::Result<()>`（接受连接直到进程结束；socket 文件已存在则先删除）。协议行为契约（P1c 客户端依赖）：
  - 每个 Request 恰好回一行 Reply；`Attach` 回 `Attached{snapshot_b64,next_offset}` 后，该连接持续收到 `Output`/`Exited` 事件行，同时仍可继续发请求（如 `Write`）
  - `Attach.from_offset > 0` 且窗口内 → snapshot 从该 offset 起；否则全量
  - broadcast 滞后（Lagged）→ 服务端发 `Error{message:"lagged; reattach"}` 并停止转发，客户端应重新 Attach

- [ ] **Step 1: 写失败的集成测试（会话存活是本计划的灵魂测试）**

```rust
// crates/dozerd/tests/session_survival.rs
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use dozerd::registry::SessionRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

struct Client {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(sock: &PathBuf) -> Client {
        let stream = UnixStream::connect(sock).await.expect("connect dozerd");
        let (r, w) = stream.into_split();
        Client { lines: BufReader::new(r).lines(), w }
    }

    async fn send(&mut self, req: &Request) {
        self.w.write_all(encode_line(req).as_bytes()).await.expect("send");
    }

    async fn recv(&mut self) -> Reply {
        let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
            .await
            .expect("reply within 5s")
            .expect("io ok")
            .expect("stream open");
        decode_line(&line).expect("valid reply")
    }
}

fn b64(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn from_b64(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).expect("valid b64")
}

#[tokio::test]
async fn session_survives_client_disconnect() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    // 等 socket 就绪
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 客户端 1：建会话（echo hello 后 cat 保持存活），attach 应看到 hello
    let mut c1 = Client::connect(&sock).await;
    c1.send(&Request::CreateSession {
        name: "主线".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "echo hello; cat".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80,
        rows: 24,
    })
    .await;
    let Reply::Created { session } = c1.recv().await else { panic!("expect Created") };
    let sid = session.id;

    c1.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;
    let mut seen = Vec::new();
    // Attached 的 snapshot 可能尚未含 hello（竞态），继续吃 Output 直到看到
    loop {
        match c1.recv().await {
            Reply::Attached { snapshot_b64, .. } => seen.extend(from_b64(&snapshot_b64)),
            Reply::Output { data_b64, .. } => seen.extend(from_b64(&data_b64)),
            other => panic!("unexpected: {other:?}"),
        }
        if seen.windows(5).any(|w| w == b"hello") {
            break;
        }
    }

    // 客户端 1 断连（模拟关窗/崩溃）
    drop(c1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 客户端 2：重连——会话必须还在、滚屏必须还有 hello（会话存活语义）
    let mut c2 = Client::connect(&sock).await;
    c2.send(&Request::ListSessions).await;
    let Reply::Sessions { sessions } = c2.recv().await else { panic!("expect Sessions") };
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].alive, "session must survive client disconnect");

    c2.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;
    let Reply::Attached { snapshot_b64, .. } = c2.recv().await else { panic!("expect Attached") };
    let snap = from_b64(&snapshot_b64);
    assert!(snap.windows(5).any(|w| w == b"hello"), "scrollback survives disconnect");

    // attach 状态下写入：cat 回显 roundtrip
    c2.send(&Request::Write { session_id: sid.clone(), data_b64: b64("roundtrip\n") }).await;
    let mut echoed = Vec::new();
    loop {
        match c2.recv().await {
            Reply::Ok => continue, // Write 的应答
            Reply::Output { data_b64, .. } => {
                echoed.extend(from_b64(&data_b64));
                if echoed.windows(9).any(|w| w == b"roundtrip") {
                    break;
                }
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    // kill 后收到 Exited；ListSessions 显示 alive=false
    c2.send(&Request::Kill { session_id: sid.clone() }).await;
    let mut exited = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(200), c2.recv()).await {
            Ok(Reply::Exited { .. }) => {
                exited = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    assert!(exited, "should receive Exited after kill");

    server.abort();
    let _ = std::fs::remove_file(&sock);
}

#[tokio::test]
async fn unknown_session_returns_error_reply() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut c = Client::connect(&sock).await;
    c.send(&Request::Attach { session_id: "ghost".into(), from_offset: 0 }).await;
    let Reply::Error { message } = c.recv().await else { panic!("expect Error") };
    assert!(message.contains("ghost"));
    server.abort();
    let _ = std::fs::remove_file(&sock);
}
```

`crates/dozerd/Cargo.toml` 增 `base64.workspace = true`。

Run: `cargo test -p dozerd --test session_survival`
Expected: FAIL —— `server` 模块不存在。

- [ ] **Step 2: 实现 server**

```rust
// crates/dozerd/src/server.rs
use crate::registry::SessionRegistry;
use crate::session::{SessionEvent, SessionSpec};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

pub async fn serve(socket: &Path, registry: Arc<SessionRegistry>) -> Result<()> {
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "dozerd 监听中");
    loop {
        let (stream, _) = listener.accept().await?;
        let registry = registry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, registry).await {
                tracing::debug!(error = %e, "连接结束");
            }
        });
    }
}

async fn handle_conn(stream: UnixStream, registry: Arc<SessionRegistry>) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    // attach 状态：订阅 + 会话 id
    let mut sub: Option<(String, broadcast::Receiver<SessionEvent>)> = None;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break }; // 客户端断连：直接退出，不动会话
                let reply = match decode_line::<Request>(&line) {
                    Err(e) => Reply::Error { message: format!("协议错误: {e}") },
                    Ok(req) => match req {
                        Request::ListSessions => Reply::Sessions { sessions: registry.list() },
                        Request::CreateSession { name, command, args, cwd, cols, rows } => {
                            match registry.create(SessionSpec { name, command, args, cwd, cols, rows }) {
                                Ok(s) => Reply::Created { session: s.info() },
                                Err(e) => Reply::Error { message: e.to_string() },
                            }
                        }
                        Request::Attach { session_id, from_offset } => match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => {
                                let rx = s.subscribe();
                                let (snap, next) = match s.read_from(from_offset) {
                                    Some(tail) if from_offset > 0 => (tail, s.snapshot().1),
                                    _ => s.snapshot(),
                                };
                                sub = Some((session_id.clone(), rx));
                                Reply::Attached {
                                    session_id,
                                    snapshot_b64: B64.encode(&snap),
                                    next_offset: next,
                                }
                            }
                        },
                        Request::Write { session_id, data_b64 } => match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => match B64.decode(&data_b64) {
                                Err(e) => Reply::Error { message: format!("base64: {e}") },
                                Ok(data) => match s.write(&data) {
                                    Ok(()) => Reply::Ok,
                                    Err(e) => Reply::Error { message: e.to_string() },
                                },
                            },
                        },
                        Request::Resize { session_id, cols, rows } => match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => match s.resize(cols, rows) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error { message: e.to_string() },
                            },
                        },
                        Request::Kill { session_id } => match registry.kill(&session_id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error { message: e.to_string() },
                        },
                    },
                };
                w.write_all(encode_line(&reply).as_bytes()).await?;
            }
            ev = async {
                match &mut sub {
                    Some((_, rx)) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if sub.is_some() => {
                let (sid, _) = sub.as_ref().expect("sub checked");
                let sid = sid.clone();
                match ev {
                    Ok(SessionEvent::Output { data, offset }) => {
                        let reply = Reply::Output { session_id: sid, data_b64: B64.encode(&data), offset };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                    }
                    Ok(SessionEvent::Exited { code }) => {
                        let reply = Reply::Exited { session_id: sid, code };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                        sub = None;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let reply = Reply::Error { message: "lagged; reattach".into() };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                        sub = None;
                    }
                    Err(broadcast::error::RecvError::Closed) => { sub = None; }
                }
            }
        }
    }
    Ok(())
}
```

`lib.rs` 增 `pub mod server;`。

- [ ] **Step 3: 跑集成测试**

Run: `cargo test -p dozerd`
Expected: 全 PASS（单元 10 + 集成 2）。灵魂断言必须绿：`session must survive client disconnect`、`scrollback survives disconnect`。

- [ ] **Step 4: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): UDS 服务端 + 会话存活集成测试——断连不掉会话、滚屏可回放

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: main 接线——可运行的 dozerd

**Files:**
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes: `server::serve`、`registry::SessionRegistry`、`dozer_core::paths::socket_path`。
- Produces: `dozerd [--socket <path>]` 可执行；Ctrl-C 优雅退出并清理 socket 文件。P1c 的 GUI 以 `dozer_core::paths::socket_path()` 为默认连接地址。

- [ ] **Step 1: 改写 main**

```rust
// crates/dozerd/src/main.rs（整体替换）
use anyhow::Result;
use dozerd::registry::SessionRegistry;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 极简参数解析：仅 --socket <path>（daemon 不引 clap，保持轻）
    let mut args = std::env::args().skip(1);
    let mut socket: PathBuf = dozer_core::paths::socket_path();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => {
                socket = args.next().map(PathBuf::from).unwrap_or(socket);
            }
            "--version" => {
                println!("dozerd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("未知参数: {other}（支持 --socket <path> / --version）");
                std::process::exit(2);
            }
        }
    }

    let registry = Arc::new(SessionRegistry::new());
    let serve = dozerd::server::serve(&socket, registry);
    tokio::select! {
        r = serve => r?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("收到 Ctrl-C，退出");
            let _ = std::fs::remove_file(&socket);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: 冒烟验证**

Run（依次三个终端命令，同一 shell 内用后台方式模拟）:

```bash
cargo build -p dozerd
./target/aarch64-apple-darwin/debug/dozerd --version
(./target/aarch64-apple-darwin/debug/dozerd --socket /tmp/dozerd-smoke.sock &) && sleep 1 && \
  printf '{"type":"list_sessions"}\n' | nc -U /tmp/dozerd-smoke.sock -w 1; \
  pkill -f dozerd-smoke && rm -f /tmp/dozerd-smoke.sock
```

Expected: `--version` 打印 `dozerd 0.1.0`；nc 收到一行 `{"type":"sessions","sessions":[]}`。

- [ ] **Step 3: 全量回归 + Commit**

Run: `cargo build && cargo test && cargo clippy --all-targets 2>&1 | tail -3`
Expected: 全绿无警告。

```bash
git add crates/dozerd
git commit -m "feat(dozerd): main 接线——默认 socket_path、--socket 覆盖、Ctrl-C 清理

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **规格覆盖**：需求 4"会话存活"由 Task 5 灵魂测试直接断言（断连→重连→滚屏在）；§4 终端数据面（裸 PTY + 字节环形缓冲 + 客户端回放坐标系）由 Task 2/3 落实；IPC JSON Lines 由 Task 1/5 落实；死会话滚屏可查（验收/审阅回放的地基）由 Task 4 落实。resume id 持久化与 daemon 自重启恢复显式归 P1f，不在本计划。
2. **占位符扫描**：无 TBD/TODO；所有步骤含完整代码与命令。
3. **类型一致性**：`SessionInfo{id,name,command,cwd,alive,created_ms}`、`Request/Reply` 各变体字段、`SessionEvent::{Output{data,offset},Exited{code}}`、`serve(&Path, Arc<SessionRegistry>)`、`snapshot()->(Vec<u8>,u64)` 在任务间交叉引用一致；`_b64` 命名约定全文统一。
