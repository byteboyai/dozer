# Dozer P1c：终端 GUI 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `dozer` GUI 成为 dozerd 的第一个真实客户端：iced_winit 集成壳（spike B 的生产化）+ 四栏工作区骨架 + 终端 pane（alacritty_terminal 状态机回放 daemon 字节流）+ 会话 tabs 与键盘输入。**关 app 重开，会话与滚屏还在——会话存活第一次以 GUI 形态可见。**

**Architecture:** 三层：`dozer-client`（UDS 协议客户端库，headless 可测）→ `dozer-app` 的 `TerminalModel`（alacritty_terminal Term 状态机，headless 可测）→ iced 渲染与事件层（integration 壳，人工验收）。逻辑全部下沉到可 headless 测试的层，GUI 层只做绘制与事件翻译。

**Tech Stack:** iced_winit/iced_wgpu/iced_widget 0.14（spike B 底座生产化）· alacritty_terminal（Term + vte 解析）· tokio · dozer-core::protocol（P1b 契约）

**规格来源:** 规格 §3 需求 1/4、§7 四栏；P1a spike GO 报告（specs/2026-07-15-spike-report-webview.md）；**P1b 终审承接项**（本计划 Task 1 强制承接：M3 offset 不变量护栏、M4 越界回退测试）。

## Global Constraints

- Rust 2024；核心不引入 Node/Python；mac 先发但不用 AppKit 专属 API。
- GUI 架构 = **iced_winit 自持 event loop + iced_wgpu**（P1a spike B 的 GO 裁决；P1d 将在同一壳上叠 webview，不得回退到高层 `iced::application`）。
- 主题 ByteBoy2077 token（值取自规格 §7）：bg `#0a0e16`、面板 `#0e1620`、终端底 `#08141d`、卡片 `#12202a`、边框 `#1c3440`、主文字 `#FFE5B4`、正文 `#9AB4C4`、弱文字 `#6B7F8F`、金 `#F2D94E`、青 `#47DEF0`、绿 `#1AD585`、紫蓝 `#9580FF`、红 `#FF6E6E`。集中在 `dozer-app/src/theme.rs`，禁止散落硬编码。
- 终端体验验收口径 = **正确 + 不卡顿**（规格 §3 需求 1 降标），像素级打磨不设关口；**只做 tabs，不做分屏**。
- 窗口标题必须 `.with_title("Dozer")`（P1a 遗留 Minor 在此清偿）。
- wry 本计划**不引入**（P1d 的事）；但布局必须给左二预览留出规则矩形区域（P1a 移交约束 ③）。
- 中文输入经 winit `Ime::Commit` 事件转 UTF-8 字节写入会话（不做预编辑窗渲染，一期口径）。
- commit 中文 + 末尾空行后 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；每任务 `cargo test`/`cargo clippy --all-targets` 全绿；TDD 红灯即时留证（P1b 整改纪律）。
- API 漂移处理：alacritty_terminal 以本地 registry 源码为准、iced_* 以 0.14 世代为准，对齐后**验收标准不变**。

---

### Task 1: dozerd 加固——P1b 终审承接项（M3/M4）+ --socket 缺值报错

**Files:**
- Modify: `crates/dozerd/tests/session_survival.rs`（追加 2 个测试）
- Modify: `crates/dozerd/src/main.rs`（--socket 缺值报错）

**Interfaces:**
- Consumes: P1b 全部既有接口。
- Produces: 无新接口；产出两道常驻回归护栏，P1c 客户端依赖的"每字节恰好投递一次"不变量从此有测试守护。

- [ ] **Step 1: 写失败/护栏测试（M3 不变量压力护栏）**

```rust
// 追加到 crates/dozerd/tests/session_survival.rs
/// M3 护栏（P1b 终审承接）：持续输出会话上多轮中途 attach，
/// 逐事件断言字节流连续性不变量 offset - data.len() == 本连接水位。
#[tokio::test]
async fn attach_stream_offset_invariant_under_load() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 持续输出源：每 10ms 一行递增序号
    let mut c0 = Client::connect(&sock).await;
    c0.send(&Request::CreateSession {
        name: "泵".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "i=0; while [ $i -lt 400 ]; do echo line_$i; i=$((i+1)); sleep 0.01; done".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80, rows: 24,
    }).await;
    let Reply::Created { session } = c0.recv().await else { panic!("expect Created") };
    let sid = session.id;
    drop(c0);

    let mut checked_events = 0u32;
    for round in 0..12 {
        let mut c = Client::connect(&sock).await;
        c.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;
        let Reply::Attached { next_offset, .. } = c.recv().await else { panic!("expect Attached") };
        let mut watermark = next_offset;
        // 每轮消费 ~15 个事件校验不变量后断连
        for _ in 0..15 {
            match tokio::time::timeout(Duration::from_secs(3), c.recv()).await {
                Ok(Reply::Output { data_b64, offset, .. }) => {
                    let len = from_b64(&data_b64).len() as u64;
                    assert_eq!(
                        offset - len, watermark,
                        "字节流断裂：round={round} offset={offset} len={len} watermark={watermark}"
                    );
                    watermark = offset;
                    checked_events += 1;
                }
                Ok(Reply::Exited { .. }) | Err(_) => break, // 输出源跑完即止
                Ok(other) => panic!("unexpected: {other:?}"),
            }
        }
        drop(c);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(checked_events >= 60, "压力不足：仅校验 {checked_events} 个事件");
    server.abort();
    let _ = std::fs::remove_file(&sock);
}
```

- [ ] **Step 2: 写 M4 越界回退测试**

```rust
/// M4（P1b 终审承接）：from_offset 已被环形缓冲逐出 → 回退全量快照。
/// 用小于缓冲窗口起点的 offset 无法直接构造（1MiB 逐出成本高），
/// 改用协议语义等价路径：from_offset 大于 total（越界）同样走"否则全量"分支。
#[tokio::test]
async fn attach_from_offset_out_of_window_falls_back_to_full_snapshot() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut c = Client::connect(&sock).await;
    c.send(&Request::CreateSession {
        name: "t".into(), command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf fallback_marker; cat".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80, rows: 24,
    }).await;
    let Reply::Created { session } = c.recv().await else { panic!() };
    // 等输出落缓冲
    tokio::time::sleep(Duration::from_millis(400)).await;
    // 越界 offset → 应回退全量（snapshot 含 marker），且 next_offset 一致可用
    c.send(&Request::Attach { session_id: session.id.clone(), from_offset: u64::MAX }).await;
    let Reply::Attached { snapshot_b64, next_offset, .. } = c.recv().await else { panic!() };
    let snap = from_b64(&snapshot_b64);
    assert!(snap.windows(15).any(|w| w == b"fallback_marker"), "越界必须回退全量快照");
    assert_eq!(next_offset, snap.len() as u64, "全量回退时 next_offset == 快照长度（窗口未逐出场景）");
    c.send(&Request::Kill { session_id: session.id }).await;
    server.abort();
    let _ = std::fs::remove_file(&sock);
}
```

Run: `cargo test -p dozerd --test session_survival`
Expected: 新测试要么直接 PASS（护栏性质，实现已正确），要么暴露真 bug——**任一失败都不许放宽断言**，修 server 到绿为止。红灯/绿灯输出均贴报告。

- [ ] **Step 3: --socket 缺值改报错（P1b M6 顺手清偿）**

`crates/dozerd/src/main.rs` 中 `"--socket"` 分支改为：

```rust
"--socket" => match args.next() {
    Some(p) => socket = PathBuf::from(p),
    None => {
        eprintln!("--socket 需要一个路径参数");
        std::process::exit(2);
    }
},
```

Run: `./target/aarch64-apple-darwin/debug/dozerd --socket; echo exit=$?`（先 `cargo build -p dozerd`）
Expected: stderr 提示 + `exit=2`。

- [ ] **Step 4: 全量回归 + Commit**

Run: `cargo test --workspace 2>&1 | tail -1 && cargo clippy --all-targets 2>&1 | tail -1`
Expected: 42 passed（40+2）、clippy 无警告。

```bash
git add crates/dozerd
git commit -m "加固(dozerd)：offset 不变量压力护栏 + 越界回退测试（P1b 终审承接）+ --socket 缺值报错

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: dozer-client——UDS 协议客户端库

**Files:**
- Create: `crates/dozer-client/Cargo.toml`
- Create: `crates/dozer-client/src/lib.rs`
- Create: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::*`；`dozer_core::paths::socket_path`。
- Produces（GUI 层唯一的 daemon 入口，后续任务照此签名）：
  - `dozer_client::Client::new(socket: PathBuf) -> Client`（不连接，惰性）
  - `async fn list(&self) -> Result<Vec<SessionInfo>>`
  - `async fn create(&self, name:&str, command:&str, args:&[String], cwd:&str, cols:u16, rows:u16) -> Result<SessionInfo>`
  - `async fn write(&self, id:&str, data:&[u8]) -> Result<()>`
  - `async fn resize(&self, id:&str, cols:u16, rows:u16) -> Result<()>`
  - `async fn kill(&self, id:&str) -> Result<()>`
  - `async fn attach(&self, id:&str, from_offset:u64) -> Result<(Vec<u8> /*snapshot*/, u64 /*next*/, tokio::sync::mpsc::UnboundedReceiver<TermEvent>)>`——内部占用一条专用连接并 spawn 读取任务
  - `enum TermEvent { Output(Vec<u8>), Exited(Option<i32>), Lagged, Disconnected }`（offset 已由 daemon 水位过滤保证连续，客户端不再暴露）
  - 控制类方法每次开短连接（本地 UDS，开销可忽略），无共享状态。

- [ ] **Step 1: 建 crate 与失败测试**

```toml
# crates/dozer-client/Cargo.toml
[package]
name = "dozer-client"
version = "0.1.0"
edition.workspace = true

[dependencies]
dozer-core = { path = "../dozer-core" }
anyhow.workspace = true
tokio.workspace = true
base64.workspace = true
tracing.workspace = true

[dev-dependencies]
dozerd = { path = "../dozerd" }
uuid.workspace = true
```

```rust
// crates/dozer-client/tests/against_real_daemon.rs
use dozer_client::{Client, TermEvent};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use std::time::Duration;

async fn start_daemon() -> std::path::PathBuf {
    let sock = std::env::temp_dir().join(format!("dozer-client-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let s = sock.clone();
    tokio::spawn(async move { dozerd::server::serve(&s, registry).await });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sock
}

#[tokio::test]
async fn full_client_lifecycle() {
    let sock = start_daemon().await;
    let c = Client::new(sock);
    assert!(c.list().await.unwrap().is_empty());

    let info = c.create("测试", "/bin/sh", &["-c".into(), "echo ready; cat".into()],
                        "/tmp", 80, 24).await.unwrap();
    assert!(info.alive);

    let (snap, _next, mut rx) = c.attach(&info.id, 0).await.unwrap();
    let mut seen = snap;
    while !seen.windows(5).any(|w| w == b"ready") {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TermEvent::Output(d))) => seen.extend(d),
            other => panic!("expect output, got {other:?}"),
        }
    }

    c.write(&info.id, b"pong\n").await.unwrap();
    let mut echoed = Vec::new();
    while !echoed.windows(4).any(|w| w == b"pong") {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TermEvent::Output(d))) => echoed.extend(d),
            other => panic!("expect echo, got {other:?}"),
        }
    }

    c.resize(&info.id, 100, 30).await.unwrap();
    c.kill(&info.id).await.unwrap();
    // kill 后 attach 流应收到 Exited
    let mut exited = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(TermEvent::Exited(_))) => { exited = true; break; }
            Ok(Some(_)) => continue,
            _ => continue,
        }
    }
    assert!(exited);
    assert!(!c.list().await.unwrap()[0].alive);
}
```

Run: `cargo test -p dozer-client`
Expected: FAIL（Client 未定义）——红灯输出贴报告。

- [ ] **Step 2: 实现**

```rust
// crates/dozer-client/src/lib.rs
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, SessionInfo, decode_line, encode_line};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum TermEvent {
    Output(Vec<u8>),
    Exited(Option<i32>),
    Lagged,
    Disconnected,
}

#[derive(Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    async fn roundtrip(&self, req: &Request) -> Result<Reply> {
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(encode_line(req).as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        let line = lines.next_line().await?.ok_or_else(|| anyhow!("daemon 断开"))?;
        let reply: Reply = decode_line(&line)?;
        if let Reply::Error { message } = &reply {
            bail!("daemon 错误: {message}");
        }
        Ok(reply)
    }

    pub async fn list(&self) -> Result<Vec<SessionInfo>> {
        match self.roundtrip(&Request::ListSessions).await? {
            Reply::Sessions { sessions } => Ok(sessions),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn create(&self, name: &str, command: &str, args: &[String],
                        cwd: &str, cols: u16, rows: u16) -> Result<SessionInfo> {
        match self.roundtrip(&Request::CreateSession {
            name: name.into(), command: command.into(), args: args.to_vec(),
            cwd: cwd.into(), cols, rows,
        }).await? {
            Reply::Created { session } => Ok(session),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn write(&self, id: &str, data: &[u8]) -> Result<()> {
        match self.roundtrip(&Request::Write {
            session_id: id.into(), data_b64: B64.encode(data),
        }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        match self.roundtrip(&Request::Resize { session_id: id.into(), cols, rows }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn kill(&self, id: &str) -> Result<()> {
        match self.roundtrip(&Request::Kill { session_id: id.into() }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 专用连接 attach：返回快照 + next_offset + 事件流（后台任务持续读取）
    pub async fn attach(&self, id: &str, from_offset: u64)
        -> Result<(Vec<u8>, u64, mpsc::UnboundedReceiver<TermEvent>)> {
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(encode_line(&Request::Attach {
            session_id: id.into(), from_offset,
        }).as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        let first = lines.next_line().await?.ok_or_else(|| anyhow!("daemon 断开"))?;
        let (snapshot, next) = match decode_line::<Reply>(&first)? {
            Reply::Attached { snapshot_b64, next_offset, .. } =>
                (B64.decode(snapshot_b64.as_bytes())?, next_offset),
            Reply::Error { message } => bail!("attach 失败: {message}"),
            other => bail!("意外应答: {other:?}"),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _keep_writer = w; // 保住写半边，连接不关
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let ev = match decode_line::<Reply>(&line) {
                            Ok(Reply::Output { data_b64, .. }) => B64
                                .decode(data_b64.as_bytes())
                                .map(TermEvent::Output)
                                .unwrap_or(TermEvent::Disconnected),
                            Ok(Reply::Exited { code, .. }) => TermEvent::Exited(code),
                            Ok(Reply::Error { message }) if message.contains("lagged") =>
                                TermEvent::Lagged,
                            _ => continue,
                        };
                        let stop = matches!(ev, TermEvent::Exited(_) | TermEvent::Disconnected);
                        if tx.send(ev).is_err() || stop { break; }
                    }
                    _ => { let _ = tx.send(TermEvent::Disconnected); break; }
                }
            }
        });
        Ok((snapshot, next, rx))
    }
}
```

根 `Cargo.toml` 无需改（`crates/*` 通配）。

- [ ] **Step 3: 跑测试 + Commit**

Run: `cargo test -p dozer-client`
Expected: PASS。

```bash
git add crates/dozer-client
git commit -m "feat(client): dozer-client 协议客户端库——控制短连接 + attach 专用流

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: dozer-app 集成壳基座 + ByteBoy2077 主题 + 四栏布局

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`（换依赖：iced_winit/iced_wgpu/iced_widget/winit + dozer-client）
- Create: `crates/dozer-app/src/theme.rs`
- Create: `crates/dozer-app/src/workspace.rs`（iced 程序状态与 view）
- Modify: `crates/dozer-app/src/main.rs`（integration 壳，spike B 生产化）

**Interfaces:**
- Consumes: spike B 代码结构（`spike/iced-webview/src/main.rs` 的 Runner 状态机——去掉 wry/webview 部分照搬）；`dozer_client::Client`。
- Produces:
  - `theme::*`：`pub const BG/PANEL/TERM_BG/CARD/BORDER/CREAM/BODY/DIM/GOLD/CYAN/GREEN/PURPLE/RED: iced::Color`（值=Global Constraints 表）
  - `workspace::Workspace`（iced 程序状态）+ `workspace::Message` 枚举；`Workspace::view()` 输出四栏：左一项目栏 stub（240px，PANEL 底）/ 左二预览 stub（规则矩形，预留 P1d，显示"预览 · P1d"占位）/ 左三终端区（TERM_BG 底，本计划主战场）/ 左四 AI 栏 stub（280px）
  - main.rs 事件循环暴露挂钩点：`fn on_window_event(&mut self, event: &WindowEvent)`（Task 5/6 键盘与 resize 接线处）

- [ ] **Step 1: 迁移壳**

以 `spike/iced-webview/src/main.rs` 为底本复制到 `dozer-app/src/main.rs`，删除全部 wry/webview/scene(wgsl shader) 相关代码，保留 Runner 状态机 + iced_wgpu 渲染管线；窗口属性改 `.with_title("Dozer").with_inner_size(LogicalSize::new(1440.0, 900.0))`；`controls.rs` 的角色由新建的 `workspace.rs` 承担。

`theme.rs`（完整给出，值不得改动）：

```rust
// crates/dozer-app/src/theme.rs
use iced::Color;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

pub const BG: Color = c(0x0a, 0x0e, 0x16);
pub const PANEL: Color = c(0x0e, 0x16, 0x20);
pub const TERM_BG: Color = c(0x08, 0x14, 0x1d);
pub const CARD: Color = c(0x12, 0x20, 0x2a);
pub const BORDER: Color = c(0x1c, 0x34, 0x40);
pub const CREAM: Color = c(0xFF, 0xE5, 0xB4);
pub const BODY: Color = c(0x9A, 0xB4, 0xC4);
pub const DIM: Color = c(0x6B, 0x7F, 0x8F);
pub const GOLD: Color = c(0xF2, 0xD9, 0x4E);
pub const CYAN: Color = c(0x47, 0xDE, 0xF0);
pub const GREEN: Color = c(0x1A, 0xD5, 0x85);
pub const PURPLE: Color = c(0x95, 0x80, 0xFF);
pub const RED: Color = c(0xFF, 0x6E, 0x6E);
```

（若 `Color::from_rgb8` 非 const fn，则改用 `Color { r: 0x0a as f32 / 255.0, .. }` 展开写法，值不变。）

`workspace.rs` 骨架：

```rust
// crates/dozer-app/src/workspace.rs
use crate::theme;
use iced_widget::{column, container, row, text};

#[derive(Debug, Clone)]
pub enum Message {
    Noop, // Task 5/6 扩展
}

pub struct Workspace;

impl Workspace {
    pub fn new() -> Self { Self }

    pub fn update(&mut self, _message: Message) {}

    pub fn view(&self) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        let col1 = pane("项目 · P1e", 240.0, theme::PANEL);
        let col2 = pane("预览 · P1d", 0.0, theme::PANEL); // 0=FILL
        let col3 = pane("终端 · 本计划", 0.0, theme::TERM_BG);
        let col4 = pane("AI · P1e", 280.0, theme::PANEL);
        row![col1, col2, col3, col4].into()
    }
}
// pane(): container + 顶部标签文字，背景/边框用 theme 常量；宽 0 表示 Fill。
// 具体 helper 按 iced_widget 0.14 API 实现，验收标准见 Step 2。
```

- [ ] **Step 2: 冒烟验收（实施者口径）**

Run: `cargo build -p dozer-app && (cargo run -p dozer-app &) && sleep 6 && pkill -f "debug/dozer$" ; true`
Expected: 编译零警告；运行 6 秒无 panic。窗口视觉（四栏可辨、标题 Dozer、配色正确）留待 Task 7 用户人工验收。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(app): iced_winit 集成壳生产化 + ByteBoy2077 主题模块 + 四栏骨架

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: TerminalModel——alacritty_terminal 状态机（headless）

**Files:**
- Modify: 根 `Cargo.toml`（workspace.dependencies 增 `alacritty_terminal = "0.25"`，以 cargo add 实际版本为准）
- Modify: `crates/dozer-app/Cargo.toml`（增 alacritty_terminal）
- Create: `crates/dozer-app/src/term_model.rs`

**Interfaces:**
- Consumes: attach 字节流（`Vec<u8>`）。
- Produces（渲染层唯一数据源）：
  - `term_model::Cell { ch: char, fg: (u8,u8,u8), bg: Option<(u8,u8,u8)>, bold: bool }`
  - `term_model::TerminalModel::new(cols: u16, rows: u16) -> Self`
  - `feed(&mut self, bytes: &[u8])`（vte 解析进 Term）
  - `resize(&mut self, cols: u16, rows: u16)`
  - `visible_lines(&self) -> Vec<Vec<Cell>>`（可视网格逐行逐格）
  - `cursor(&self) -> (usize /*col*/, usize /*row*/)`
  - ANSI 16 色映射到 ByteBoy2077 系（黑→TERM_BG、白→CREAM、绿→GREEN、红→RED、蓝→PURPLE、青→CYAN、黄→GOLD，亮色同映射；默认前景 BODY）——映射表放本模块，返回 RGB 三元组避免 iced 依赖渗入。

- [ ] **Step 1: 写失败测试**

```rust
// crates/dozer-app/src/term_model.rs 尾部
#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(cells: &[Cell]) -> String {
        cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn plain_text_lands_on_first_row() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"hello dozer");
        assert_eq!(line_text(&t.visible_lines()[0]), "hello dozer");
        assert_eq!(t.cursor(), (11, 0));
    }

    #[test]
    fn newline_and_cr_move_cursor() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"one\r\ntwo");
        let lines = t.visible_lines();
        assert_eq!(line_text(&lines[0]), "one");
        assert_eq!(line_text(&lines[1]), "two");
        assert_eq!(t.cursor(), (3, 1));
    }

    #[test]
    fn sgr_red_foreground_is_mapped() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"\x1b[31mred\x1b[0m");
        let cell = &t.visible_lines()[0][0];
        assert_eq!(cell.ch, 'r');
        assert_eq!(cell.fg, (0xFF, 0x6E, 0x6E)); // ANSI 红 → 主题 RED
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"keepme");
        t.resize(60, 20);
        assert_eq!(line_text(&t.visible_lines()[0]), "keepme");
        assert_eq!(t.visible_lines().len(), 20);
    }

    #[test]
    fn utf8_cjk_occupies_two_columns() {
        let mut t = TerminalModel::new(40, 10);
        t.feed("你好".as_bytes());
        let l = &t.visible_lines()[0];
        assert_eq!(l[0].ch, '你');
        assert_eq!(l[2].ch, '好'); // 宽字符占两格，第 1 格为 spacer
        assert_eq!(t.cursor(), (4, 0));
    }
}
```

Run: `cargo test -p dozer-app term_model`
Expected: FAIL（类型未定义）——红灯留证。

- [ ] **Step 2: 实现（对齐 alacritty_terminal 实际 API）**

实现语义锚点（以本地 registry 源码为准对齐，测试语义不变）：

```rust
// crates/dozer-app/src/term_model.rs（结构示意，字段/方法名以 alacritty_terminal 实际版本对齐）
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

pub struct TerminalModel {
    term: Term<VoidListener>,
    parser: Processor,
    size: TermSize, // alacritty 的尺寸类型（cols/screen_lines），版本对齐
}

// new(): Term::new(Config::default(), &size, VoidListener)
// feed(): for b in bytes { self.parser.advance(&mut self.term, ...) }
//         （0.24+ 的 advance 接受 &[u8] 批量；以实际签名为准）
// resize(): self.term.resize(new_size)
// visible_lines(): 遍历 term.grid() 的可视区 row/col，
//         cell.c 为字符；cell.flags 含 WIDE_CHAR_SPACER（spacer 格 ch 置 ' '）；
//         fg/bg 经 vte::ansi::Color → 本模块 ANSI→主题映射表转 RGB；
//         Named(NamedColor::Foreground) → BODY 色，Background → None
// cursor(): term.grid().cursor.point → (column.0, line.0 转可视行号)
```

ANSI 16 色映射表（完整值，禁止漂移）：

| ANSI | RGB | | ANSI | RGB |
|------|-----|-|------|-----|
| Black | (0x08,0x14,0x1d) | | BrightBlack | (0x6B,0x7F,0x8F) |
| Red | (0xFF,0x6E,0x6E) | | BrightRed | (0xFF,0x8E,0x8E) |
| Green | (0x1A,0xD5,0x85) | | BrightGreen | (0x4A,0xE5,0xA5) |
| Yellow | (0xF2,0xD9,0x4E) | | BrightYellow | (0xFF,0xF3,0xB0) |
| Blue | (0x95,0x80,0xFF) | | BrightBlue | (0xB5,0xA5,0xFF) |
| Magenta | (0xFF,0x3D,0xCC) | | BrightMagenta | (0xFF,0x6D,0xDC) |
| Cyan | (0x47,0xDE,0xF0) | | BrightCyan | (0x87,0xEE,0xF8) |
| White | (0xFF,0xE5,0xB4) | | BrightWhite | (0xFF,0xF5,0xD4) |
| 默认前景 | (0x9A,0xB4,0xC4) | | 默认背景 | None |

- [ ] **Step 3: 跑测试 + Commit**

Run: `cargo test -p dozer-app`
Expected: 5 个 PASS（含 CJK 宽字符）。

```bash
git add Cargo.toml Cargo.lock crates/dozer-app
git commit -m "feat(app): TerminalModel——alacritty_terminal 状态机 + ANSI→ByteBoy2077 色映射

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: 终端渲染 + 键盘/IME 输入翻译（keymap headless 可测）

**Files:**
- Create: `crates/dozer-app/src/keymap.rs`
- Create: `crates/dozer-app/src/term_view.rs`
- Modify: `crates/dozer-app/src/workspace.rs`（终端 pane 接 TerminalModel）
- Modify: `crates/dozer-app/src/main.rs`（窗口事件 → keymap → Message）

**Interfaces:**
- Consumes: `TerminalModel::{visible_lines,cursor}`；winit `KeyEvent`/`Ime`。
- Produces:
  - `keymap::key_to_bytes(key: &winit::keyboard::Key, modifiers: &winit::keyboard::ModifiersState) -> Option<Vec<u8>>`——字符键→UTF-8；Enter→`\r`；Backspace→`0x7f`；Tab→`\t`；Esc→`0x1b`；方向键→CSI `\x1b[A/B/C/D`；Ctrl+字母→控制码（Ctrl+C→0x03 等）
  - `keymap::ime_commit_to_bytes(text: &str) -> Vec<u8>`（UTF-8 原样）
  - `term_view::view(model: &TerminalModel, focused: bool) -> Element<Message>`——逐行 rich_text/span 渲染（等宽字体 iced Font::MONOSPACE），bold 加粗，光标格反色（focused 时实心 CREAM 底 TERM_BG 字，否则描边）；行数×列数由 pane 像素尺寸与字号（13px，行高 1.4）换算，换算函数 `term_view::grid_size(width_px, height_px) -> (cols, rows)` 公开且有单测

- [ ] **Step 1: keymap 失败测试**

```rust
// crates/dozer-app/src/keymap.rs 尾部
#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    #[test]
    fn printable_char_passes_utf8() {
        assert_eq!(key_to_bytes(&Key::Character("a".into()), &ModifiersState::empty()), Some(b"a".to_vec()));
    }

    #[test]
    fn enter_is_cr_backspace_is_del() {
        assert_eq!(key_to_bytes(&Key::Named(NamedKey::Enter), &ModifiersState::empty()), Some(vec![b'\r']));
        assert_eq!(key_to_bytes(&Key::Named(NamedKey::Backspace), &ModifiersState::empty()), Some(vec![0x7f]));
    }

    #[test]
    fn arrows_are_csi() {
        assert_eq!(key_to_bytes(&Key::Named(NamedKey::ArrowUp), &ModifiersState::empty()), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes(&Key::Named(NamedKey::ArrowLeft), &ModifiersState::empty()), Some(b"\x1b[D".to_vec()));
    }

    #[test]
    fn ctrl_c_is_etx() {
        assert_eq!(key_to_bytes(&Key::Character("c".into()), &ModifiersState::CONTROL), Some(vec![0x03]));
    }

    #[test]
    fn ime_commit_is_raw_utf8() {
        assert_eq!(ime_commit_to_bytes("你好"), "你好".as_bytes().to_vec());
    }
}
```

grid_size 单测（放 term_view.rs）：

```rust
#[test]
fn grid_size_from_pixels() {
    // 字号 13px 等宽：单元格宽 ≈ 7.8px（0.6em），行高 ≈ 18.2px（1.4）
    let (cols, rows) = grid_size(780.0, 546.0);
    assert!((95..=105).contains(&cols), "cols={cols}");
    assert!((28..=32).contains(&rows), "rows={rows}");
}
```

Run: `cargo test -p dozer-app keymap term_view`
Expected: FAIL——红灯留证。

- [ ] **Step 2: 实现 keymap + term_view + 接线**

keymap 按测试实现（纯函数，无外部状态）。term_view：每行一个 `rich_text`（或按 0.14 实际 API 用 `text` span 序列），行容器 `column`，等宽 `Font::MONOSPACE` 13px；光标绘制：把光标格的 span 换成反色。workspace 的终端 pane 调 `term_view::view`；main.rs 的 `WindowEvent::KeyboardInput`/`Ime::Commit` 在终端聚焦时经 keymap 转 bytes 发 `Message::TermInput(Vec<u8>)`（本任务先把 bytes 回灌 `TerminalModel::feed` 做本地 echo 验证渲染管线，daemon 接线在 Task 6）。

- [ ] **Step 3: 跑测试 + 冒烟 + Commit**

Run: `cargo test -p dozer-app && cargo clippy --all-targets 2>&1 | tail -1`
Expected: 全绿（term_model 5 + keymap 5 + grid_size 1）。

```bash
git add crates/dozer-app
git commit -m "feat(app): 终端渲染 term_view + 键盘/IME keymap（headless 全测）

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: 会话接线——tabs、attach 流、输入直达 daemon、启动恢复

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `dozer_client::{Client, TermEvent}`；Task 4/5 全部。
- Produces（行为契约，Task 7 人工验收依据）：
  - 启动时 `Client::list()`：为每个 alive 会话开 tab 并 attach(from_offset:0)（快照喂 TerminalModel）——**GUI 级会话恢复**
  - daemon 连不上 → 自动 spawn `dozerd`（`std::process::Command`，同目录二进制或 PATH），1 秒后重试，仍失败则终端区显示错误文案（RED）
  - tab 栏：每会话一个 tab（名称 + alive 状态点 GREEN/DIM）；`＋` 新建（默认 `$SHELL` 或 `/bin/zsh`，cwd=$HOME）；tab 关闭 = detach（drop 事件流），**不 kill**
  - 键盘/IME 输入 → `client.write(session_id, bytes)`（不再本地 echo——回显走 PTY 真实回路）
  - 终端 pane 尺寸变化 → `grid_size` 换算 → `TerminalModel::resize` + `client.resize`
  - TermEvent::Exited → tab 状态点变 DIM + 尾行打印 `[会话已结束]`（CREAM on CARD）
  - tokio runtime 与 winit 循环共存：main 持 `tokio::runtime::Runtime`，事件流经 `runtime.spawn` + `std::sync::mpsc`（或 winit EventLoopProxy 自定义事件）送回 UI 线程——**禁止在 UI 线程 block_on 网络 IO**（attach 快照除外，容忍一次性 block_on）

- [ ] **Step 1: 实现接线**（本任务为集成层，无新 headless 测试；正确性由 Task 1-5 的测试 + Task 7 人工验收覆盖）

实现要点（结构锚点）：

```rust
// workspace.rs 新增
pub struct SessionTab {
    pub info: SessionInfo,
    pub model: TerminalModel,
    pub alive: bool,
}
pub enum Message {
    TermInput(Vec<u8>),            // keymap 产物 → client.write
    TermOutput(usize, Vec<u8>),    // 事件流 → tabs[i].model.feed
    SessionExited(usize),
    SelectTab(usize),
    NewTab,
    PaneResized { cols: u16, rows: u16 },
    DaemonError(String),
}
```

main.rs：启动序列 = 连 daemon（失败则 spawn dozerd 重试）→ list → 逐会话 attach → 快照 feed → 事件流经 EventLoopProxy 送 `Message::TermOutput`。

- [ ] **Step 2: 编译冒烟 + 手动脚本验证**

Run: `cargo build -p dozer-app 2>&1 | tail -1 && cargo clippy --all-targets 2>&1 | tail -1`
Expected: 零警告。

Run（有头环境下实施者可做的最强验证；无头则记录跳过，留 Task 7）:
`pkill -f dozerd; (cargo run -p dozer-app &); sleep 8; pgrep -f "debug/dozerd" && echo "✓ GUI 自动拉起了 dozerd"; pkill -f "debug/dozer"`
Expected: dozerd 被 GUI 自动拉起。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(app): 会话接线——tabs/attach 流/输入直达 daemon/启动恢复/自动拉起 dozerd

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: 端到端人工验收 + 规格回填

**Files:**
- Create: `docs/superpowers/specs/2026-07-16-p1c-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（§3 需求 1/4 标注 GUI 级达成）

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录；P1d 的起点状态。

- [ ] **Step 1: 用户人工验收（协调者陪同，逐项 ✓/✗）**

```markdown
# P1c 人工验收清单（用户实机执行）
1. `cargo run -p dozer-app`：窗口标题 Dozer、四栏可辨、ByteBoy2077 配色正确
2. dozerd 未运行时启动 app → 自动拉起 daemon，无错误
3. ＋ 新建 tab → shell 会话：输入 `ls -G`、`echo 你好`（中文 IME）回显正确、颜色正确
4. 跑 `top` 或 `vim` 再退出：全屏程序渲染不花屏（正确性口径，允许瑕疵记录）
5. **灵魂项**：关掉 app → 重新打开 → tab 自动恢复、滚屏完整、可继续输入
6. 新 tab 跑 `claude`：交互正常（这是 Dozer 第一次真实驱动 agent）
7. 窗口拖拽缩放：终端 reflow 不崩、cols/rows 跟随
8. 体感：滚动输出（`cat` 大文件）不卡顿
```

- [ ] **Step 2: 结果落档 + 规格回填 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-16-p1c-acceptance.md`（含 ✗ 项与处置）；规格 §3 需求 1、4 条目末尾追加"P1c 达成（<日期>），验收记录见 specs/2026-07-16-p1c-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1c 终端 GUI 人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **规格覆盖**：需求 1（终端：tabs/shell 输出/中文/颜色/reflow）由 T4/5/6/7 覆盖；需求 4 的 GUI 级呈现（关 app 会话不丢）由 T6 启动恢复 + T7 灵魂项覆盖；P1b 终审承接 M3/M4 在 T1；P1a 遗留（.title、--socket 报错）在 T3/T1 清偿；spike GO 架构约束在 Global Constraints 锁死。预览/验收闭环/审阅不在本计划（P1d/P1e/P1f）。
2. **占位符扫描**：T4 Step 2 与 T6 Step 1 为"语义锚点 + 对齐指引"形态（alacritty/iced 0.14 API 漂移风险高，逐字代码反而误导），验收标准均为可执行测试/清单，不构成 TBD。其余任务代码完整。
3. **类型一致性**：`TermEvent` 变体、`Client` 方法签名、`TerminalModel::{feed,resize,visible_lines,cursor}`、`Cell{ch,fg,bg,bold}`、`grid_size`、theme 常量名在任务间交叉引用一致；ANSI 映射表与 theme.rs 值一致（RED=0xFF6E6E、GREEN=0x1AD585 等）。
