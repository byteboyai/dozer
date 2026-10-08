# Usage 面板可行性探针(O12)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建一个只在测试里存在的"最小 host + fake 服务"夹具,让 Usage 面板在其中跑完 8 个验收场景,并产出缺口报告,回答"第二个消费者需要 host 提供什么"。

**Architecture:** 夹具放在 `crates/dozer-app/src/usage_probe/`(`#[cfg(test)]`,**host 一侧**,不在 `extensions/` 里,所以不触碰面板边界门禁)。夹具由四块组成:讲真实 UDS 协议的假 dozerd、记录推送的假 webview 表面、`PanelHost` 假实现、以及 `ProbeHost`(照搬 `App` 对 Usage 做的事,并把每一件"面板自己做不了、由 host 代做"的事记进 `duties` 日志)。场景测试分"期望成立"与"特征化(钉住现状缺口)"两类。**零生产代码改动**,仅 `main.rs` 加一行 `mod` 声明。

**Tech Stack:** Rust(`dozer-app` 内部测试)、`tokio` UDS、`dozer_core::protocol`、`bytegit` 的 `TempRepo`(testutil)、Python `unittest`(`scripts/audit`)。

**Spec:** `docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md`(D1 无头夹具 / D2 fake UDS / D3 报告放 `docs/dozer-v2/bytehost-H0/`,均按推荐)。

## Global Constraints

- **零生产代码改动。** 唯一允许改的非新增文件:`crates/dozer-app/src/main.rs`(加 `#[cfg(test)] mod usage_probe;`)、三份文档(Task 6)。任务结束时 `git diff --stat main -- crates/` 里除 `main.rs` 外只能有新增文件。
- **不得引入新 trait**(`UsageSource` 等只作为缺口写进报告)。
- **日志**:夹具里不用 `tracing`/`eprintln!`(`scripts/check-log-scope.sh` 门禁)。测试断言失败信息用 `assert!` 消息即可。
- **字体/UI**:本计划不画 UI;`content_pane`/`list_pane` 只被构造、不渲染。
- **测试超时**:所有等待 channel 的地方用 `recv_timeout(TIMEOUT)`(`TIMEOUT = 5s`,来自 `panel_host::testing`),不得无限阻塞。
- **工作目录**:在独立 worktree `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe` 里做,**所有命令与文件路径都带这个前缀**(项目历史上实现者会漂到 main 分支)。worktree 必须是 `dozer` 的**同级目录**,因为被跟踪的 `.cargo/config.toml` 用 `../byteui`、`../bytegit` 相对路径做 patch。
- **提交**:每个 Task 末尾提交;`git add` 只加指定路径,提交前 `git status --short` 看全貌。提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

1. **夹具与真实 host 漂移**:`ProbeHost::tick` 是 `App::take_usage_content_script` 的手抄版,`app.rs` 改了夹具不会报错。→ Task 3 加漂移守卫测试(读 `app.rs` 源码,断言关键调用的先后顺序)。
2. **UDS 路径过长 / 残留 socket**:macOS `sockaddr_un` 路径上限约 104 字节。→ Task 1 用 `tempfile` 短前缀 + 固定文件名 `d.sock`,并有"同时起两个 FakeDozerd 互不干扰"的测试。
3. **测试挂死**:假服务 `HangUp`/不应答时客户端等待。→ 所有接收用 `recv_timeout`;`HangUp` 靠关连接让客户端立刻拿到"daemon 断开"。
4. **特征化测试被误当成"期望行为"**:S4/S6/S7 钉住的是**缺口现状**。→ 测试名带 `characterizes_`,断言消息写明"这是缺口 Gx,修复后应翻转本测试"。
5. **git 提交数依赖仓库状态**:→ 用 `TempRepo` 造固定 1 个提交的仓库,不读真实仓库。

---

### Task 1: 假 dozerd(讲真实 UDS 协议)

**Files:**
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/mod.rs`
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/fake_dozerd.rs`
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/main.rs`(在 `mod transcript;` 之后加一行)

**Interfaces:**
- Produces(`fake_dozerd.rs`,均 `pub(crate)`):
  - `type UsageRows = Vec<(dozer_core::protocol::ConversationSummary, dozer_core::protocol::UsagePayload)>;`
  - `enum Behavior { Rows(UsageRows), ErrorReply(String), HangUp }`(`Clone`)
  - `struct FakeDozerd`;`FakeDozerd::start(rt: &tokio::runtime::Runtime, behavior: Behavior) -> FakeDozerd`;`fn socket(&self) -> &std::path::Path`;`fn cwds(&self) -> Vec<String>`(按到达顺序记录每次 `GetUsageSummary` 的 `cwd`)
- Produces(`mod.rs`,`pub(crate)`):`fn summary(id: &str, agent: AgentKind, last_ts_ms: u64) -> ConversationSummary`;`fn payload(turns: u32) -> UsagePayload`;`const NOW_MS: u64 = 1_790_000_000_000;`

- [ ] **Step 1: 建 worktree 与分支,带上规格和计划**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-usage-probe -b probe/usage-o12 main
cp docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md ../dozer-usage-probe/docs/superpowers/specs/
cp docs/superpowers/plans/2026-10-08-usage-panel-probe.md ../dozer-usage-probe/docs/superpowers/plans/
cd ../dozer-usage-probe
git add docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md docs/superpowers/plans/2026-10-08-usage-panel-probe.md
git status --short
git commit -m "docs(probe): Usage 面板 O12 探针规格与执行计划

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git status --short` 只显示这两个新增文件;提交成功。

- [ ] **Step 2: 写失败的测试**

`usage_probe/mod.rs`:

```rust
//! Usage 面板可行性探针(O12)的夹具,规格 `docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md`。
//! 只在测试里存在;放在 host 一侧而不是 `extensions/`,因为它扮演的是 host。

mod fake_dozerd;

use dozer_core::protocol::{AgentKind, ConversationSummary, UsagePayload};
use std::collections::BTreeSet;

pub(crate) const NOW_MS: u64 = 1_790_000_000_000;

pub(crate) fn summary(id: &str, agent: AgentKind, last_ts_ms: u64) -> ConversationSummary {
    ConversationSummary {
        conversation_id: id.into(),
        agent,
        file_path: format!("/t/{id}.jsonl"),
        title: id.into(),
        first_ts: last_ts_ms,
        last_ts: last_ts_ms,
        turn_count: 3,
    }
}

pub(crate) fn payload(turns: u32) -> UsagePayload {
    UsagePayload {
        turns,
        tool_calls: 2,
        mutating_tool_calls: 1,
        files_touched: BTreeSet::new(),
        tokens_in: 100,
        tokens_out: 50,
        tokens_cache_read: 10,
        tokens_cache_write: 5,
    }
}
```

`usage_probe/fake_dozerd.rs`(先只放测试与类型桩,让测试因 `start` 未实现而失败):

```rust
//! 假 dozerd:在临时目录里监听一个 UDS,讲真实协议(`dozer_core::protocol` 的按行 JSON),
//! 只实现 `GetUsageSummary`。

use dozer_core::protocol::{ConversationSummary, UsagePayload};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(crate) type UsageRows = Vec<(ConversationSummary, UsagePayload)>;

#[derive(Clone)]
pub(crate) enum Behavior {
    Rows(UsageRows),
    ErrorReply(String),
    HangUp,
}

pub(crate) struct FakeDozerd {
    socket: PathBuf,
    cwds: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
}

impl FakeDozerd {
    pub(crate) fn start(_rt: &tokio::runtime::Runtime, _behavior: Behavior) -> Self {
        unimplemented!()
    }
    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }
    pub(crate) fn cwds(&self) -> Vec<String> {
        self.cwds.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_probe::{NOW_MS, payload, summary};
    use crate::panel_host::testing::{TIMEOUT, runtime};
    use dozer_core::protocol::AgentKind;

    fn client_for(d: &FakeDozerd) -> dozer_client::Client {
        dozer_client::Client::new(d.socket().to_path_buf())
    }

    #[test]
    fn serves_preset_rows_through_the_real_client_and_records_the_cwd() {
        let rt = runtime();
        let rows = vec![(summary("c1", AgentKind::Claude, NOW_MS), payload(4))];
        let d = FakeDozerd::start(&rt, Behavior::Rows(rows));
        let got = rt
            .block_on(tokio::time::timeout(TIMEOUT, client_for(&d).get_usage_summary("/proj", None)))
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.conversation_id, "c1");
        assert_eq!(got[0].1.turns, 4);
        assert_eq!(d.cwds(), vec!["/proj".to_string()]);
    }

    #[test]
    fn hang_up_and_error_reply_surface_as_client_errors() {
        let rt = runtime();
        for behavior in [Behavior::HangUp, Behavior::ErrorReply("boom".into())] {
            let d = FakeDozerd::start(&rt, behavior);
            let res = rt
                .block_on(tokio::time::timeout(TIMEOUT, client_for(&d).get_usage_summary("/p", None)))
                .unwrap();
            assert!(res.is_err(), "假服务不应答/报错时客户端必须拿到 Err");
        }
    }

    #[test]
    fn two_servers_do_not_share_a_socket() {
        let rt = runtime();
        let a = FakeDozerd::start(&rt, Behavior::Rows(vec![]));
        let b = FakeDozerd::start(&rt, Behavior::Rows(vec![]));
        assert_ne!(a.socket(), b.socket());
        assert!(a.socket().as_os_str().len() < 100, "UDS 路径过长会 bind 失败");
    }
}
```

在 `main.rs` 的 `mod transcript;` 下一行加:

```rust
#[cfg(test)]
mod usage_probe;
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe::fake_dozerd 2>&1 | tail -20`
Expected: 编译通过,三个测试 FAIL(`not implemented`)。首次构建较慢。

- [ ] **Step 4: 实现 `start`**

把 `fake_dozerd.rs` 顶部的 `use` 与 `start` 换成:

```rust
use dozer_core::protocol::{
    ConversationSummary, Reply, Request, UsagePayload, decode_line, encode_line,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
```

```rust
    pub(crate) fn start(rt: &tokio::runtime::Runtime, behavior: Behavior) -> Self {
        // 短前缀 + 固定短文件名:macOS 的 UDS 路径上限约 104 字节。
        let dir = tempfile::Builder::new().prefix("dzp").tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let listener = {
            // `UnixListener::bind` 要在 runtime 上下文里注册 reactor。
            let _guard = rt.enter();
            UnixListener::bind(&socket).unwrap()
        };
        let cwds = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&cwds);
        rt.spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let Ok(Some(line)) = lines.next_line().await else {
                    continue;
                };
                let Ok(Request::GetUsageSummary { cwd, .. }) = decode_line::<Request>(&line)
                else {
                    continue;
                };
                seen.lock().unwrap().push(cwd);
                let reply = match &behavior {
                    Behavior::Rows(rows) => Reply::UsageSummary { rows: rows.clone() },
                    Behavior::ErrorReply(message) => Reply::Error {
                        message: message.clone(),
                    },
                    // 不写应答直接丢弃本连接 = 对端关闭,客户端得到 "daemon 断开"。
                    Behavior::HangUp => continue,
                };
                let _ = w.write_all(encode_line(&reply).as_bytes()).await;
            }
        });
        Self {
            socket,
            cwds,
            _dir: dir,
        }
    }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe::fake_dozerd 2>&1 | tail -10`
Expected: 3 passed。若 `ConversationSummary`/`UsagePayload` 缺 `Clone` 导致 `rows.clone()` 编译失败,**不要改生产类型**:改成在 `Behavior::Rows` 里存 `Arc<UsageRows>` 并手动构造应答所需的克隆——先看 `crates/dozer-core/src/protocol.rs` 的 derive,再决定。

- [ ] **Step 6: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
git add crates/dozer-app/src/main.rs crates/dozer-app/src/usage_probe/
git status --short
git commit -m "test(probe): Usage 探针的假 dozerd(真实 UDS 协议)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: 假 webview 表面与 PanelHost 假实现

**Files:**
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/surface.rs`
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/fake_panel_host.rs`
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/mod.rs`(加两行 `mod`)

**Interfaces:**
- Consumes: `crate::extensions::usage::{encode_usage_push, UsageViewPayload}`;`crate::panel_host::PanelHost`
- Produces(`pub(crate)`):
  - `FakeSurface`(`Default`):`attach(&mut self)`、`detach(&mut self)`、`is_attached(&self) -> bool`、`receive(&mut self, envelope_json: String)`、`count(&self) -> usize`、`kinds(&self) -> Vec<String>`(每条 envelope 的 `payload.kind`)
  - `FakePanelHost`(`Default`):实现 `PanelHost`;`collapse_buttons_asked(&self) -> u32`

- [ ] **Step 1: 写失败的测试**

`surface.rs`:

```rust
//! 假 webview 表面:记录 host 通过 `evaluate_script` 推过来的每条 envelope,并能模拟"销毁/重建"。

#[derive(Default)]
pub(crate) struct FakeSurface {
    attached: bool,
    received: Vec<String>,
}

impl FakeSurface {
    pub(crate) fn attach(&mut self) {
        unimplemented!()
    }
    pub(crate) fn detach(&mut self) {
        unimplemented!()
    }
    pub(crate) fn is_attached(&self) -> bool {
        unimplemented!()
    }
    pub(crate) fn receive(&mut self, _envelope_json: String) {
        unimplemented!()
    }
    pub(crate) fn count(&self) -> usize {
        unimplemented!()
    }
    pub(crate) fn kinds(&self) -> Vec<String> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::usage::{UsageViewPayload, encode_usage_push};

    #[test]
    fn records_envelopes_and_reports_their_payload_kind() {
        let mut s = FakeSurface::default();
        assert!(!s.is_attached());
        s.attach();
        assert!(s.is_attached());
        s.receive(encode_usage_push(UsageViewPayload::Empty));
        assert_eq!(s.count(), 1);
        assert_eq!(s.kinds(), vec!["empty".to_string()]);
        s.detach();
        assert!(!s.is_attached());
        assert_eq!(s.count(), 1, "detach 不清空已收到的记录");
    }
}
```

`fake_panel_host.rs`:

```rust
//! `PanelHost` 的最小假实现(从 `panel_host.rs` 测试里的 `FakeHost` 演化而来,那份是私有的)。

use crate::app::{HoverId, PanelKind};
use crate::panel_host::PanelHost;
use iced_widget::core::Element;
use std::cell::Cell;

#[derive(Default)]
pub(crate) struct FakePanelHost {
    collapse_buttons_asked: Cell<u32>,
}

impl FakePanelHost {
    pub(crate) fn collapse_buttons_asked(&self) -> u32 {
        self.collapse_buttons_asked.get()
    }
}

impl PanelHost for FakePanelHost {
    fn hover_progress(&self, _id: HoverId) -> f32 {
        0.0
    }
    fn hover_tooltip_ready(&self, _id: HoverId) -> bool {
        false
    }
    fn list_collapsed(&self, _kind: PanelKind) -> bool {
        false
    }
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        _kind: PanelKind,
        _collapsed: bool,
        _hover_id: HoverId,
        _tooltip_collapse: &'a str,
        _tooltip_expand: &'a str,
        _on_select: M,
        _on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        self.collapse_buttons_asked
            .set(self.collapse_buttons_asked.get() + 1);
        iced_widget::Space::new().into()
    }
    fn window_size(&self) -> (f32, f32) {
        (800.0, 600.0)
    }
    fn last_cursor(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::usage::{self, WorkspaceState};
    use iced_widget::core::{Border, Length};

    #[test]
    fn usage_views_build_against_the_fake_host() {
        let host = FakePanelHost::default();
        let ws = WorkspaceState::default();
        let _content = usage::content_pane(&host, &ws, Length::Fill, Border::default());
        let _list = usage::list_pane(&ws, Length::Fill, Border::default());
        assert_eq!(host.collapse_buttons_asked(), 1);
    }
}
```

在 `mod.rs` 的 `mod fake_dozerd;` 下加:

```rust
mod fake_panel_host;
mod surface;
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe:: 2>&1 | tail -15`
Expected: `surface` 的测试 FAIL(`not implemented`);`fake_panel_host` 的测试 PASS(它不依赖桩)。若 `list_pane` 签名与上面不符,按编译器报错对齐到 `crates/dozer-app/src/extensions/usage/view.rs` 的真实签名。

- [ ] **Step 3: 实现 `FakeSurface`**

```rust
impl FakeSurface {
    pub(crate) fn attach(&mut self) {
        self.attached = true;
    }
    pub(crate) fn detach(&mut self) {
        self.attached = false;
    }
    pub(crate) fn is_attached(&self) -> bool {
        self.attached
    }
    pub(crate) fn receive(&mut self, envelope_json: String) {
        self.received.push(envelope_json);
    }
    pub(crate) fn count(&self) -> usize {
        self.received.len()
    }
    pub(crate) fn kinds(&self) -> Vec<String> {
        self.received
            .iter()
            .map(|json| {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                v["payload"]["kind"].as_str().unwrap().to_string()
            })
            .collect()
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe:: 2>&1 | tail -10`
Expected: 全部 passed(Task 1 的 3 个 + 本任务 2 个)。

- [ ] **Step 5: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
git add crates/dozer-app/src/usage_probe/
git status --short
git commit -m "test(probe): 假 webview 表面与 PanelHost 假实现

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: ProbeHost(代做 host 职责并记账)与漂移守卫

**Files:**
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/host.rs`
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/mod.rs`(加 `mod host;`)

**Interfaces:**
- Consumes: Task 1 的 `FakeDozerd`;Task 2 的 `FakeSurface`、`FakePanelHost`;`crate::extensions::usage::{WorkspaceState, WebviewPushState, Message, on_activate, update, current_view_payload, encode_usage_push, parse_usage_event, UsageWebviewEvent}`;`crate::panel_host::{ActivationCtx, PanelIo}`;`dozer_client::Client`
- Produces(`pub(crate)`):
  - `enum Duty { BuildActivationCtx, RouteLoaded, DropOtherProject, InterceptCollapse, InterceptHover, FramePush, DowngradeReady, SkipWhileLoading, ParseIpcAndMarkReady }`(`Debug, Clone, Copy, PartialEq, Eq, Hash`);`Duty::host_site(self) -> &'static str`(真实 host 里对应的位置,给报告用)
  - `struct ProbeHost { pub ws: WorkspaceState, pub push: WebviewPushState, pub surface: FakeSurface, pub duties: Vec<Duty> }`
  - `ProbeHost::new(project_id: i64) -> Self`
  - `ProbeHost::activate(&mut self, rt: &tokio::runtime::Runtime, dozerd: &FakeDozerd, path: Option<std::path::PathBuf>) -> std::sync::mpsc::Receiver<Message>`
  - `ProbeHost::deliver(&mut self, msg: Message)`
  - `ProbeHost::tick(&mut self)`
  - `ProbeHost::webview_ready(&mut self)`
  - `ProbeHost::duty_set(&self) -> std::collections::HashSet<Duty>`

- [ ] **Step 1: 写失败的测试**

`host.rs`(先放类型与桩 + 测试):

```rust
//! `ProbeHost`:扮演 `App` 对 Usage 面板做的那部分事。每一件"面板自己做不了、由 host 代做"的事都记进
//! `duties`,这份日志就是缺口报告的原始数据。**逻辑从真实 host 手抄而来**,见各方法注释里的出处;
//! `drift_guard` 测试负责在出处改动时报警。

use super::fake_dozerd::FakeDozerd;
use super::surface::FakeSurface;
use crate::extensions::usage::{self, Message, WebviewPushState, WorkspaceState};
use crate::panel_host::{ActivationCtx, PanelIo};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Duty {
    BuildActivationCtx,
    RouteLoaded,
    DropOtherProject,
    InterceptCollapse,
    InterceptHover,
    FramePush,
    DowngradeReady,
    SkipWhileLoading,
    ParseIpcAndMarkReady,
}

impl Duty {
    /// 真实 host 里承担这件事的位置(读代码得到,2026-10-08)。
    pub(crate) fn host_site(self) -> &'static str {
        match self {
            Duty::BuildActivationCtx => "app/update.rs:5251",
            Duty::RouteLoaded => "app/update.rs:1621 (with_project)",
            Duty::DropOtherProject => "app/update.rs:1621 (with_project 路由到别的 Workspace)",
            Duty::InterceptCollapse => "app/update.rs:1626",
            Duty::InterceptHover => "app/update.rs:1629",
            Duty::FramePush => "app/app.rs:1242 take_usage_content_script",
            Duty::DowngradeReady => "app/app.rs:1250",
            Duty::SkipWhileLoading => "app/app.rs:1255",
            Duty::ParseIpcAndMarkReady => "runtime.rs:705 + app/update.rs:261",
        }
    }
}

pub(crate) struct ProbeHost {
    pub ws: WorkspaceState,
    pub push: WebviewPushState,
    pub surface: FakeSurface,
    pub duties: Vec<Duty>,
    project_id: i64,
}

impl ProbeHost {
    pub(crate) fn new(_project_id: i64) -> Self {
        unimplemented!()
    }
    pub(crate) fn activate(
        &mut self,
        _rt: &tokio::runtime::Runtime,
        _dozerd: &FakeDozerd,
        _path: Option<PathBuf>,
    ) -> mpsc::Receiver<Message> {
        unimplemented!()
    }
    pub(crate) fn deliver(&mut self, _msg: Message) {
        unimplemented!()
    }
    pub(crate) fn tick(&mut self) {
        unimplemented!()
    }
    pub(crate) fn webview_ready(&mut self) {
        unimplemented!()
    }
    pub(crate) fn duty_set(&self) -> HashSet<Duty> {
        self.duties.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::usage::UsageViewPayload;

    #[test]
    fn tick_without_a_surface_downgrades_ready_and_pushes_nothing() {
        let mut h = ProbeHost::new(1);
        h.push.set_ready(true);
        h.tick();
        assert_eq!(h.surface.count(), 0);
        assert!(h.duty_set().contains(&Duty::DowngradeReady));
        assert!(h.push.pending_push(&UsageViewPayload::Empty).is_none(), "ready 已被降级");
    }

    #[test]
    fn webview_ready_then_tick_pushes_once_and_dedups() {
        let mut h = ProbeHost::new(1);
        h.surface.attach();
        h.webview_ready();
        h.tick();
        h.tick();
        assert_eq!(h.surface.kinds(), vec!["empty".to_string()]);
    }

    /// 漂移守卫:`tick` 是 `App::take_usage_content_script` 的手抄版。真实函数里这几步的先后顺序一变,
    /// 本测试就失败,提醒同步夹具(并在报告里记一笔"host 行为变了")。
    #[test]
    fn drift_guard_tick_mirrors_take_usage_content_script() {
        let src = include_str!("../app/app.rs");
        let start = src
            .find("pub fn take_usage_content_script")
            .expect("app.rs 里找不到 take_usage_content_script");
        let body = &src[start..];
        let body = &body[..body.find("pub fn take_codehealth_content_script").unwrap()];
        let order = [
            "available_webview_ids.contains",
            "set_ready(false)",
            "active_workspace()",
            "usage.loading()",
            "current_view_payload",
            "pending_push",
            "encode_usage_push",
            "mark_sent",
        ];
        let mut at = 0;
        for needle in order {
            let found = body[at..]
                .find(needle)
                .unwrap_or_else(|| panic!("take_usage_content_script 里 `{needle}` 不在预期位置之后,夹具需同步"));
            at += found;
        }
    }
}
```

在 `mod.rs` 加 `mod host;`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe::host 2>&1 | tail -15`
Expected: 前两个测试 FAIL(`not implemented`);`drift_guard` PASS(只读源码)。若 `drift_guard` 失败,说明 `app.rs` 与读代码时不同,以当前源码为准改 `order` 数组并在 commit 信息里写明。

- [ ] **Step 3: 实现**

```rust
impl ProbeHost {
    pub(crate) fn new(project_id: i64) -> Self {
        Self {
            ws: WorkspaceState::default(),
            push: WebviewPushState::default(),
            surface: FakeSurface::default(),
            duties: Vec::new(),
            project_id,
        }
    }

    /// 对应 `app/update.rs:5251`:host 构造 `ActivationCtx` 再调面板的 `on_activate`。
    pub(crate) fn activate(
        &mut self,
        rt: &tokio::runtime::Runtime,
        dozerd: &FakeDozerd,
        path: Option<PathBuf>,
    ) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = dozer_client::Client::new(dozerd.socket().to_path_buf());
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        let ctx = ActivationCtx {
            project_id: self.project_id,
            project_path: path,
            io,
        };
        self.duties.push(Duty::BuildActivationCtx);
        usage::on_activate(&mut self.ws, Some(&ctx));
        rx
    }

    /// 对应 `app/update.rs:1621-1633` 的路由:`Loaded` 按 `project_id` 路由(这里只有一个 Workspace,
    /// 非本项目的等价于"路由到别的 Workspace",对本面板即丢弃);`ToggleListCollapse`/`Hover` 由 host
    /// 拦截,不进面板的 `update`。
    pub(crate) fn deliver(&mut self, msg: Message) {
        match msg {
            Message::Loaded(project_id, ..) if project_id != self.project_id => {
                self.duties.push(Duty::DropOtherProject);
            }
            msg @ Message::Loaded(..) => {
                self.duties.push(Duty::RouteLoaded);
                usage::update(&mut self.ws, msg);
            }
            Message::ToggleListCollapse => self.duties.push(Duty::InterceptCollapse),
            Message::Hover(..) => self.duties.push(Duty::InterceptHover),
            msg => usage::update(&mut self.ws, msg),
        }
    }

    /// 对应 `App::take_usage_content_script`(`app/app.rs:1242`):每帧声明式比较"该显示什么"与"上次送达什么"。
    pub(crate) fn tick(&mut self) {
        self.duties.push(Duty::FramePush);
        if !self.surface.is_attached() {
            self.push.set_ready(false);
            self.duties.push(Duty::DowngradeReady);
            return;
        }
        if self.ws.loading() {
            self.duties.push(Duty::SkipWhileLoading);
            return;
        }
        let desired = usage::current_view_payload(&self.ws);
        let Some(payload) = self.push.pending_push(&desired) else {
            return;
        };
        let envelope = usage::encode_usage_push(payload.clone());
        self.push.mark_sent(payload);
        self.surface.receive(envelope);
    }

    /// 对应 `runtime.rs:705` 的 IPC 识别 + `app/update.rs:261` 的 `Ready` 处理。
    pub(crate) fn webview_ready(&mut self) {
        self.duties.push(Duty::ParseIpcAndMarkReady);
        let event = usage::parse_usage_event(r#"{"kind":"ready"}"#).unwrap();
        if matches!(event, usage::UsageWebviewEvent::Ready) {
            self.push.set_ready(true);
        }
    }

    pub(crate) fn duty_set(&self) -> HashSet<Duty> {
        self.duties.iter().copied().collect()
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe:: 2>&1 | tail -10`
Expected: 全部 passed。

- [ ] **Step 5: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
git add crates/dozer-app/src/usage_probe/
git status --short
git commit -m "test(probe): ProbeHost 代做 host 职责并记账,附漂移守卫

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 验收场景 S1–S8

**Files:**
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/scenarios.rs`
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/crates/dozer-app/src/usage_probe/mod.rs`(加 `mod scenarios;`)

**Interfaces:**
- Consumes: Tasks 1–3 的全部产出;`bytegit::testutil::TempRepo`(dev-dependency 已开 `testutil`;若路径不是 `bytegit::testutil::TempRepo`,以 `crates/dozer-app/src/extensions/usage/mod.rs` 现有测试里的 import 为准,用 `grep -rn "TempRepo" crates/dozer-app/src | head -3` 查)
- Produces: 8 个场景测试(S1–S8)。**不产出供其他任务调用的 API。**

**命名约定:** 期望成立的场景以 `sN_` 开头;钉住缺口现状的以 `sN_characterizes_gM_` 开头,断言消息里写"缺口 GM,修复后应翻转本测试"。

- [ ] **Step 1: 写场景测试(这一步一次性写出全部,随后逐个确认)**

`scenarios.rs`:

```rust
//! 规格 §2.3 的验收场景 S1–S8。`characterizes_` 开头的测试钉住的是**缺口现状**,不是期望行为。

use super::fake_dozerd::{Behavior, FakeDozerd};
use super::fake_panel_host::FakePanelHost;
use super::host::{Duty, ProbeHost};
use super::{NOW_MS, payload, summary};
use crate::conversation::ConversationMeta;
use crate::extensions::usage::{self, ConversationUsage, Message};
use crate::panel_host::testing::{TIMEOUT, runtime};
use dozer_core::protocol::AgentKind;
use iced_widget::core::{Border, Length};
use std::collections::BTreeMap;

const PROJECT: i64 = 7;

fn two_agent_rows() -> Behavior {
    Behavior::Rows(vec![
        (summary("c1", AgentKind::Claude, NOW_MS), payload(4)),
        (summary("c2", AgentKind::Codex, NOW_MS), payload(2)),
    ])
}

fn row(agent: AgentKind, turns: u32) -> (ConversationMeta, ConversationUsage) {
    (
        ConversationMeta::from_summary(&summary("x", agent, NOW_MS)),
        ConversationUsage::from(&payload(turns)),
    )
}

fn repo_with_one_commit() -> bytegit::testutil::TempRepo {
    let repo = bytegit::testutil::TempRepo::new();
    repo.commit_file("a.txt", "x", "c1");
    repo
}

/// S1:切入 → 假服务返回 2 个会话 → `Loaded` → 恰好推出一条 payload。
#[test]
fn s1_activate_loads_rows_and_pushes_one_payload() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.webview_ready();

    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    assert!(host.ws.loading(), "切入立即置“统计中”");
    host.tick();
    assert_eq!(host.surface.count(), 0, "加载中不推送");

    host.deliver(rx.recv_timeout(TIMEOUT).expect("Loaded 应在超时内到达"));
    assert!(!host.ws.loading());
    assert_eq!(host.ws.rows().len(), 2);
    assert_eq!(host.ws.git_commits(), 1, "TempRepo 恰有 1 个提交");

    host.tick();
    host.tick();
    assert_eq!(host.surface.kinds(), vec!["all_agents".to_string()], "未变化不重发");
    assert_eq!(dozerd.cwds(), vec![repo.path().to_string_lossy().into_owned()]);

    // 视图也能在假 host 上构造(规格 §1:面板只靠 PanelHost 契约就能画)。
    let fake = FakePanelHost::default();
    let _c = usage::content_pane(&fake, &host.ws, Length::Fill, Border::default());
    let _l = usage::list_pane(&host.ws, Length::Fill, Border::default());
    assert_eq!(fake.collapse_buttons_asked(), 1);

    // 这个场景里 host 替面板做的事,就是缺口报告“host 能力清单”的底稿。
    let duties = host.duty_set();
    for d in [
        Duty::BuildActivationCtx,
        Duty::RouteLoaded,
        Duty::FramePush,
        Duty::SkipWhileLoading,
        Duty::ParseIpcAndMarkReady,
    ] {
        assert!(duties.contains(&d), "S1 应触发 host 职责 {d:?}");
    }
}

/// S2:切换 agent 筛选 → payload 变化,且只在变化时推一次。
#[test]
fn s2_agent_filter_changes_payload_once() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.webview_ready();
    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
    host.tick();

    host.deliver(Message::AgentFilterSet(Some(AgentKind::Claude)));
    host.tick();
    host.tick();
    assert_eq!(
        host.surface.kinds(),
        vec!["all_agents".to_string(), "single_agent".to_string()]
    );
}

/// S3:项目没有路径 → 置“统计中”但不发请求(迁移前行为)。
#[test]
fn s3_no_project_path_marks_loading_but_never_calls_the_service() {
    let rt = runtime();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    let rx = host.activate(&rt, &dozerd, None);
    assert!(host.ws.loading());
    assert!(rx.recv_timeout(std::time::Duration::from_millis(200)).is_err());
    assert!(dozerd.cwds().is_empty());
}

/// S4:服务不可达/报错 → 面板不 panic,结果与“没有数据”无法区分。
#[test]
fn s4_characterizes_g4_service_failure_looks_like_no_data() {
    for behavior in [Behavior::HangUp, Behavior::ErrorReply("boom".into())] {
        let rt = runtime();
        let repo = repo_with_one_commit();
        let dozerd = FakeDozerd::start(&rt, behavior);
        let mut host = ProbeHost::new(PROJECT);
        host.surface.attach();
        host.webview_ready();
        let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
        host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
        assert!(!host.ws.loading(), "失败后仍要退出“统计中”");
        assert!(host.ws.rows().is_empty());
        assert_eq!(host.ws.git_commits(), 1, "git 这条独立数据源不受 dozerd 失败影响");
        host.tick();
        assert_eq!(
            host.surface.kinds(),
            vec!["empty".to_string()],
            "缺口 G4:服务失败被 unwrap_or_default 吞掉,用户看到的是“无数据”而非错误;修复后应翻转本测试"
        );
    }
}

/// S5:webview 销毁重建 → 同一内容重发。
#[test]
fn s5_webview_rebuild_resends_the_same_content() {
    let rt = runtime();
    let repo = repo_with_one_commit();
    let dozerd = FakeDozerd::start(&rt, two_agent_rows());
    let mut host = ProbeHost::new(PROJECT);
    host.surface.attach();
    host.webview_ready();
    let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
    host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
    host.tick();
    assert_eq!(host.surface.count(), 1);

    host.surface.detach();
    host.tick();
    host.surface.attach();
    host.tick();
    assert_eq!(host.surface.count(), 1, "新实例 ready 之前不推送");

    host.webview_ready();
    host.tick();
    assert_eq!(
        host.surface.kinds(),
        vec!["all_agents".to_string(), "all_agents".to_string()],
        "Ready 清空 last_sent,强制重发当前内容"
    );
}

/// S6:较早的结果后到 → 面板无法识别过期;跨项目结果面板自己也不区分。
#[test]
fn s6_characterizes_g3_panel_cannot_tell_stale_results() {
    let mut host = ProbeHost::new(PROJECT);
    let newer = Message::Loaded(
        PROJECT,
        vec![row(AgentKind::Claude, 1), row(AgentKind::Codex, 2)],
        0,
        BTreeMap::new(),
    );
    let older = Message::Loaded(PROJECT, vec![row(AgentKind::Claude, 1)], 0, BTreeMap::new());
    host.deliver(newer);
    host.deliver(older);
    assert_eq!(
        host.ws.rows().len(),
        1,
        "缺口 G3:后到的旧结果覆盖了新结果——Loaded 没有序号/revision,面板自己无从判断;修复后应翻转本测试"
    );

    // 跨项目:host 的 `with_project` 负责丢弃;面板 `update` 本身不看 project_id。
    host.deliver(Message::Loaded(99, vec![row(AgentKind::Claude, 1)], 0, BTreeMap::new()));
    assert!(host.duty_set().contains(&Duty::DropOtherProject));
    let mut bare = usage::WorkspaceState::default();
    usage::update(
        &mut bare,
        Message::Loaded(99, vec![row(AgentKind::Claude, 1)], 0, BTreeMap::new()),
    );
    assert_eq!(
        bare.rows().len(),
        1,
        "缺口 G3:没有 host 路由时,面板会收下任何项目的结果"
    );
}

/// S7:`Message` 里有只能由 host 处理的变体,误投就 panic。
#[test]
#[should_panic(expected = "由内核拦截处理")]
fn s7_characterizes_g5_toggle_list_collapse_must_never_reach_the_panel() {
    let mut ws = usage::WorkspaceState::default();
    usage::update(&mut ws, Message::ToggleListCollapse);
}

#[test]
#[should_panic(expected = "由内核拦截处理")]
fn s7_characterizes_g5_hover_must_never_reach_the_panel() {
    use crate::app::{HoverId, PanelKind};
    let mut ws = usage::WorkspaceState::default();
    usage::update(&mut ws, Message::Hover(HoverId::list_collapse(PanelKind::Usage), true));
}

/// S7 的另一面:夹具里 host 拦截后,这两条消息不会进面板。
#[test]
fn s7_host_intercepts_collapse_and_hover() {
    use crate::app::{HoverId, PanelKind};
    let mut host = ProbeHost::new(PROJECT);
    host.deliver(Message::ToggleListCollapse);
    host.deliver(Message::Hover(HoverId::list_collapse(PanelKind::Usage), true));
    let duties = host.duty_set();
    assert!(duties.contains(&Duty::InterceptCollapse));
    assert!(duties.contains(&Duty::InterceptHover));
}

/// S8:夹具(不含本文件)不引用 `App`/`Workspace`,也不引用 `usage` 以外的任何面板。
#[test]
fn s8_probe_fixture_names_no_other_panel_and_no_app_or_workspace() {
    let files = [
        ("fake_dozerd.rs", include_str!("fake_dozerd.rs")),
        ("surface.rs", include_str!("surface.rs")),
        ("fake_panel_host.rs", include_str!("fake_panel_host.rs")),
        ("host.rs", include_str!("host.rs")),
    ];
    for (name, src) in files {
        for line in src.lines().filter(|l| l.trim_start().starts_with("use ")) {
            for panel in line.split("extensions::").skip(1) {
                let first: String = panel
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                assert_eq!(first, "usage", "{name}: 引用了别的面板 `{first}`(违反 R2): {line}");
            }
            for token in line.split(|c: char| !c.is_alphanumeric() && c != '_') {
                assert!(
                    token != "App" && token != "Workspace",
                    "{name}: 引用了 host 内核类型 `{token}`(违反 R1): {line}"
                );
            }
        }
    }
}
```

在 `mod.rs` 加 `mod scenarios;`。

- [ ] **Step 2: 跑测试,逐个处理红灯**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage_probe::scenarios 2>&1 | tail -40`
Expected: 全部 passed。红灯的处理规则:
- **编译错误**(类型/可见性/签名不符):对齐到真实代码,**不改生产代码**;若某个面板符号是私有的(如 `ConversationUsage` 字段、`list_pane` 可见性),在报告里记为缺口 G9"面板内部类型未导出",并改用公开路径绕过。
- **S1/S2/S5 断言失败**:先判断是夹具手抄错了(对照 `app/app.rs:1242-1266`),还是面板行为与读代码时不同。后者是重要发现:改断言到真实行为,并把差异写进报告的"意外发现"一节。
- **S4/S6 的特征化断言失败**:说明预判的缺口不成立——这正是探针要回答的。把断言翻成"期望行为"(去掉 `characterizes_`),并在报告里把对应 G 判为 REFUTED。

- [ ] **Step 3: 全套 usage_probe 测试 + 面板既有测试一起跑,确认没有互相影响**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && cargo test -p dozer-app usage 2>&1 | tail -8`
Expected: 全部 passed(包含 `extensions::usage` 既有测试与 `panel_host` 里的 usage 测试)。

- [ ] **Step 4: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
git add crates/dozer-app/src/usage_probe/
git status --short
git commit -m "test(probe): Usage 面板 S1-S8 验收场景(含缺口特征化测试)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: 依赖边清单(A/B/C 分类)

**Files:**
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/scripts/audit/unit_edges.py`
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/scripts/audit/test_unit_edges.py`

**Interfaces:**
- Consumes: `scripts/audit/edges.py` 的 `SRC`、`unit_of`、`strip_comments`、`edges_of`
- Produces: `unit_edges.classify(mod: str, sym: str) -> str`;`unit_edges.scan(unit: str) -> list[tuple[str, str, str, str, int]]`(`(class, mod, sym, files, count)`);CLI `python3 scripts/audit/unit_edges.py ext:usage` 输出 Markdown 表,存在 `?` 类时退出码 1。

**分类定义(与规格 §2.4 一致,外加 S/T/U):**
`A` 已有契约(`panel_host::*` 除 `testing`);`B` 通用 UI 件,应进 SDK 或 byteui;`C` 领域类型,归属待定(O7);`S` 数据服务依赖(无接缝);`T` 仅测试;`U` 框架库,不计;`?` 未分类。

- [ ] **Step 1: 写失败的测试**

`test_unit_edges.py`:

```python
import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("unit_edges", os.path.join(os.path.dirname(__file__), "unit_edges.py"))
unit_edges = importlib.util.module_from_spec(spec); spec.loader.exec_module(unit_edges)


class Classify(unittest.TestCase):
    def test_contract_types_are_A(self):
        self.assertEqual(unit_edges.classify("panel_host", "PanelHost"), "A")
        self.assertEqual(unit_edges.classify("panel_host", "ActivationCtx"), "A")

    def test_panel_host_testing_is_T(self):
        self.assertEqual(unit_edges.classify("panel_host", "testing"), "T")

    def test_generic_ui_is_B_and_domain_is_C_and_service_is_S(self):
        self.assertEqual(unit_edges.classify("chrome", "homespace"), "B")
        self.assertEqual(unit_edges.classify("app", "HoverId"), "B")
        self.assertEqual(unit_edges.classify("workspace", "agent_icon"), "B")
        self.assertEqual(unit_edges.classify("conversation", "ConversationMeta"), "C")
        self.assertEqual(unit_edges.classify("dozer_core", "protocol"), "C")
        self.assertEqual(unit_edges.classify("dozer_client", "Client"), "S")
        self.assertEqual(unit_edges.classify("bytegit", "Repo"), "S")

    def test_unknown_is_question_mark(self):
        self.assertEqual(unit_edges.classify("nonexistent", "Thing"), "?")


class ScanUsage(unittest.TestCase):
    def test_usage_has_no_unclassified_edges(self):
        rows = unit_edges.scan("ext:usage")
        self.assertTrue(rows, "ext:usage 应至少有一条依赖边")
        self.assertEqual([r for r in rows if r[0] == "?"], [])

    def test_usage_depends_on_workspace_helpers(self):
        mods = {(r[1], r[2]) for r in unit_edges.scan("ext:usage")}
        self.assertIn(("workspace", "agent_icon"), mods)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && python3 -m unittest scripts/audit/test_unit_edges.py 2>&1 | tail -8`
Expected: ERROR(找不到 `unit_edges.py`)。

- [ ] **Step 3: 实现**

`unit_edges.py`:

```python
#!/usr/bin/env python3
"""单元依赖边清单(O12 探针用):列出一个单元(如 ext:usage)对 crate 内其他模块与外部 crate 的依赖,
按 A/B/C/S/T/U 分类。有 `?`(未分类)时退出码 1,逼着新增依赖被显式归类。

用法: unit_edges.py ext:usage
"""
import collections, importlib.util, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(edges)

# A 已有契约 / B 通用 UI 件(应进 SDK 或 byteui)/ C 领域类型(O7)/ S 数据服务(无接缝)/ T 仅测试 / U 框架库
INTERNAL = {
    ("panel_host", "PanelHost"): "A",
    ("panel_host", "ActivationCtx"): "A",
    ("panel_host", "PanelIo"): "A",
    ("panel_host", "testing"): "T",
    ("chrome", "homespace"): "B",
    ("app", "HoverId"): "B",
    ("app", "PanelKind"): "B",
    ("workspace", "agent_icon"): "B",
    ("workspace", "agent_dot_color"): "B",
    ("conversation", "ConversationMeta"): "C",
}
EXTERNAL = {"dozer_core": "C", "dozer_client": "S", "bytegit": "S", "byteui": "B"}
EXTERNAL_RE = re.compile(r"\b(dozer_core|dozer_client|bytegit|byteui)::([A-Za-z_][A-Za-z0-9_]*)")


def classify(mod, sym):
    if (mod, sym) in INTERNAL:
        return INTERNAL[(mod, sym)]
    if mod in EXTERNAL:
        return EXTERNAL[mod]
    return "?"


def scan(unit):
    """[(class, mod, sym, files, count)],按 class、mod、sym 排序。"""
    acc = collections.defaultdict(lambda: [0, set()])
    for dp, _, fns in sorted(os.walk(edges.SRC)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dp, fn)
            rel = os.path.relpath(path, edges.SRC).replace(os.sep, "/")
            if edges.unit_of(rel) != unit:
                continue
            with open(path, encoding="utf-8") as f:
                text = f.read()
            for (mod, sym), n in edges.edges_of(text).items():
                acc[(mod, sym)][0] += n
                acc[(mod, sym)][1].add(os.path.basename(rel))
            for m in EXTERNAL_RE.finditer(edges.strip_comments(text)):
                key = (m.group(1), m.group(2))
                acc[key][0] += 1
                acc[key][1].add(os.path.basename(rel))
    rows = [(classify(m, s), m, s, ",".join(sorted(fs)), n) for (m, s), (n, fs) in acc.items()]
    return sorted(rows)


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    rows = scan(argv[1])
    print("| 类 | 模块 | 符号 | 出现文件 | 次数 |")
    print("|---|---|---|---|---|")
    for cls, mod, sym, files, n in rows:
        print(f"| {cls} | `{mod}` | `{sym}` | {files} | {n} |")
    return 1 if any(r[0] == "?" for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
```

- [ ] **Step 4: 跑测试与 CLI**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && python3 -m unittest scripts/audit/test_unit_edges.py 2>&1 | tail -6; python3 scripts/audit/unit_edges.py ext:usage; echo "exit=$?"`
Expected: 测试 OK。CLI 若退出码为 1,表里的 `?` 行就是未分类依赖:按上面的分类定义把它们加进 `INTERNAL`/`EXTERNAL`(例如新的 `crate::theme::*` 属 `B`),再跑到 `exit=0`。**把最终的表原样保存到 `/private/tmp/claude-501/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/331bcbdb-a4ad-4363-954b-2fe14d53657d/scratchpad/usage-edges.md`**,Task 6 要引用。

- [ ] **Step 5: 确认原有审计测试没被影响**

Run: `cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe && python3 -m unittest discover -s scripts/audit 2>&1 | tail -4`
Expected: OK(原 28 个 + 新增)。

- [ ] **Step 6: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
git add scripts/audit/unit_edges.py scripts/audit/test_unit_edges.py
git status --short
git commit -m "chore(audit): 单元依赖边清单(A/B/C/S/T/U 分类),供 O12 探针使用

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 缺口报告与文档回填

**Files:**
- Create: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md`(状态行 + §4 增加 G8)
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/docs/dozer-v2/bytehost-H8-evaluation.md`(末尾追加 §7)
- Modify: `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe/docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`(§7 追加第 13 项)

**Interfaces:**
- Consumes: Task 4 的测试结果、Task 5 的依赖边表、`ProbeHost.duties` 的 `host_site()` 清单。

- [ ] **Step 1: 收集原始数据**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
cargo test -p dozer-app usage_probe -- --list 2>/dev/null | grep ': test' | sort
cargo test -p dozer-app usage_probe 2>&1 | grep -E "^test |test result"
python3 scripts/audit/unit_edges.py ext:usage
```

记录:每个 `sN_*` 测试的通过/失败、特征化测试是否仍成立(成立 = 缺口被证实)、依赖边表。

- [ ] **Step 2: 写缺口报告**

`07-usage-probe-gaps.md` 必须包含以下六节,**结论一律由 Step 1 的实测结果按判定规则得出,不得凭预判填写**:

1. **一句话结论**:三选一——"能跑"/"能跑但需要下表 N 项 host 能力"/"不能跑,因为 X"。判定规则:S1、S2、S3、S5、S8 全部通过且没有为绕过可见性问题而改动生产代码 = "能跑";再按第 2 节的 host 能力数 N 写进结论。
2. **host 能力清单**:`Duty` 的 9 项,每项一行:职责、`host_site()`、面板是否能自己做(能/不能)、第二个消费者是否必须由自己的 host 提供(是/否)。**"面板不能自己做且消费者必须提供"的项数 = N。**
3. **G1–G8 判定表**:列为 `编号 | 假设 | 证据(测试名 + 文件行) | 结论`。结论取 `CONFIRMED` / `REFUTED` / `PARTIAL`。判定规则:
   - G1(webview 表面契约缺失):`Duty::FramePush`/`DowngradeReady`/`ParseIpcAndMarkReady` 三项都落在"面板不能自己做" → CONFIRMED。
   - G2(推送在 `App` 上):`drift_guard` 通过且 `tick` 逻辑只能放 host → CONFIRMED。
   - G3(过期结果路由在 host):`s6_characterizes_g3_*` 仍成立 → CONFIRMED;被翻转 → REFUTED。
   - G4(数据服务无接缝、错误被吞):`s4_characterizes_g4_*` 仍成立,且 fake UDS 是唯一不改生产代码的 fake 手段 → CONFIRMED。
   - G5(`Message` 混有 host 专属变体):两个 `s7_characterizes_g5_*` 的 `should_panic` 均成立 → CONFIRMED。
   - G6(`ConversationMeta`/`AgentKind` 属 O7):依赖边表里存在 `C` 类边 → CONFIRMED。
   - G7(分栏状态在 host):本探针夹具**没有**实现任何分栏/折叠状态也能跑通 S1–S5 → 说明面板逻辑不依赖它,但布局由 host 持有这一点来自读代码(规格 §1),判 PARTIAL 并注明"未由探针直接验证"。
   - G8(新增,规格漏列):`view.rs` 依赖 `workspace::agent_icon`/`agent_dot_color`(依赖边表 `B` 类)——Usage 的视图借用了 `Workspace` 模块里的通用 helper。依赖边表里出现这两项 → CONFIRMED。
4. **意外发现**:Task 4 Step 2 里记下的任何与读代码预期不符的现象;没有就写"无"。
5. **依赖边清单**:粘贴 Task 5 的表,并在表后按 A/B/C/S 各数一下条数。
6. **对 Q-H8 的回填建议**:按规格 §4 末尾的规则——G1–G8 中被 CONFIRMED 且归类为"缺契约/缺接缝"的条数 ≥3 → 建议"H8 范围缩小为补齐这几块契约(列出哪几块)";否则 → 维持 H8 评估的"停在 H7a"。并明确回答:O7 是否需要提前解(依据 G6)。

- [ ] **Step 3: 文档回填**

1. 规格:顶部"性质"行后加一行 `> 状态:探针已执行(2026-10-08),结论见 docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md。`;§4 表追加一行 `| G8 | Usage 视图借用 workspace::agent_icon/agent_dot_color(规格 §1 漏列) | 缺契约(通用 UI helper 住在 Workspace 模块) |`。
2. `bytehost-H8-evaluation.md` 末尾追加 `## 7. 探针结果(2026-10-08)`:两到三句话 + 指向报告,并写明 Q-H8-1 已按 A+B 裁决。
3. `dozer-v2-面板独立性与服务化要求.md` §7 追加:`13. **O12 Usage 探针已完成(2026-10-08):** <一句话结论>。夹具 `crates/dozer-app/src/usage_probe/`(仅测试),报告 `docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`。`——`<一句话结论>` 用报告第 1 节的原文。

- [ ] **Step 4: 全量验证**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
cargo test -p dozer-app 2>&1 | grep -E "test result|FAILED|failed" | head
cargo clippy -p dozer-app --all-targets 2>&1 | tail -5
cargo fmt --check -p dozer-app && echo fmt-ok
python3 -m unittest discover -s scripts/audit 2>&1 | tail -2
python3 scripts/audit/check_panel_boundary.py; echo "gate exit=$?"
bash scripts/check-log-scope.sh; echo "log-scope exit=$?"
git diff --stat main -- crates/ | tail -5
```

Expected:全部通过/无新增 clippy 警告/`fmt-ok`/两个门禁退出码 0。最后一条 `git diff --stat main -- crates/` 里**非新增**的文件只能是 `crates/dozer-app/src/main.rs`(+2 行)。若还有别的文件,逐个说明或回退——这条是"零生产代码改动"约束的验收。

- [ ] **Step 5: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-usage-probe
git add docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md docs/dozer-v2/bytehost-H8-evaluation.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md
git status --short
git commit -m "docs(probe): Usage 探针缺口报告与 H8 评估回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: 交付**

不要自行合并。向用户汇报:报告第 1 节的一句话结论、第 6 节的建议、分支名 `probe/usage-o12`、worktree 路径;合并由用户审阅后决定(项目惯例:plan 在独立分支做,审阅后合并;合并前先 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer status --short` 看 main 上有无并发 WIP)。

---

## Self-Review(已执行)

- **Spec 覆盖:** §2.1 C1–C5 → Task 2/3(C1 `FakePanelHost`、C2 `activate`、C3 `deliver`+`duties`、C4 `FakeSurface`、C5 `tick`);§2.2 fake UDS + `TempRepo` → Task 1/4;§2.3 S1–S8 → Task 4(S3 无路径、S8 隔离各有测试);§2.4 依赖边 → Task 5;§3 四项产出物 → Task 6;§4 G1–G7 → Task 6 判定表(并补 G8);§5 非目标由 Global Constraints 的"零生产代码改动/不新增 trait"落实;§6 D1–D3 按推荐。
- **占位符扫描:** 无 TBD/TODO。Task 6 的结论是数据依赖的,已给出逐条判定规则而不是留空。
- **类型一致性:** `FakeDozerd::start/socket/cwds`、`Behavior::{Rows,ErrorReply,HangUp}`、`ProbeHost::{new,activate,deliver,tick,webview_ready,duty_set}`、`Duty` 九个变体、`FakeSurface::{attach,detach,is_attached,receive,count,kinds}` 在各任务间一致;`Message::Loaded(i64, Vec<(ConversationMeta, ConversationUsage)>, u64, BTreeMap<i64,u64>)` 与 `usage/mod.rs` 一致。
- **已知未验证点(执行时以编译器为准):** `bytegit::testutil::TempRepo` 的 import 路径;`ConversationSummary`/`UsagePayload` 是否 `Clone`;`list_pane` 的确切签名;`ConversationUsage` 是否对外可构造。每处在对应步骤里写了处理规则。
