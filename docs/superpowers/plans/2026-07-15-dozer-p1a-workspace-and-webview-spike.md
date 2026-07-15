# Dozer P1a：Workspace 重构 + iced×wry 合成 Spike 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把仓库改造成 Dozer 的 Cargo workspace（boy 降级为 legacy 种子代码），并用两个 spike 验证一期头号技术风险——iced(wgpu) 窗口内叠加原生 WebView 子视图（含 ⌘K 隐藏/显示、随布局变更 bounds）。

**Architecture:** 根 Cargo.toml 变为纯 workspace；现有 byteboy 代码原样移入 `crates/legacy-boy`（只读种子，不再扩展）；新增 `dozer-core`（共享路径/类型）与 `dozerd`/`dozer-app`/`dozer-hook` 三个可执行骨架；`spike/` 下两个一次性验证程序（winit+wry 先证 wry 路径，iced_winit 集成再证生产路径）。

**Tech Stack:** Rust 2024 edition · iced 0.14（含 iced_winit/iced_wgpu 集成 shell）· wry（WKWebView 子视图）· winit 0.30 · tokio 1 · directories · tracing · anyhow/thiserror

**规格来源:** `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（终稿）；后续计划 P1b–P1g 依赖本计划的 crate 布局与 spike 结论。

## Global Constraints

- 平台：macOS Apple Silicon 先发；技术选型全走 Rust 跨平台生态，**禁止 Swift/AppKit 专属绑定**（wry/winit 的平台抽象层内部除外）。
- Rust edition 2024；workspace resolver "3"。
- 核心依赖**不得引入 Node.js/Python 运行时**。
- 二进制命名：`dozerd`（daemon）、`dozer`（GUI app）、`dozer-hook`（hook 小工具）。`boy` 保留可编译但已废弃，不再添加任何功能。
- 主题 ByteBoy2077 核心色（spike 用到背景即可）：窗口底 `#0a0e16`，金 `#F2D94E`，奶油文字 `#FFE5B4`。
- 每个任务以通过测试 + git commit 结束；commit message 中文，落款 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`。
- iced/wry API 若与本文代码有签名漂移：以 crates.io 对应版本的官方示例为准对齐（iced 仓库 tag 0.14 的 `examples/integration`、wry 仓库 `examples/wgpu.rs`），**不得因漂移改变任务的验收标准**。

---

### Task 1: 仓库 workspace 化，byteboy 移入 legacy 种子位

**Files:**
- Modify: `Cargo.toml`（根，改为纯 workspace）
- Create: `crates/legacy-boy/Cargo.toml`
- Move: `src/` → `crates/legacy-boy/src/`（git mv，代码零改动）
- Modify: `CLAUDE.md`（重写为 Dozer 现实）

**Interfaces:**
- Consumes: 现有 byteboy 包（bin 名 `boy`）。
- Produces: workspace 根；成员 `crates/*`、`spike/*`；后续任务均以 `cargo <cmd> -p <crate>` 操作。

- [ ] **Step 1: 记录现状基线**

Run: `cargo test 2>&1 | tail -3 && cargo build 2>&1 | tail -1`
Expected: 编译通过（现有测试全绿或记下既有失败作为基线）。

- [ ] **Step 2: 移动包体**

```bash
mkdir -p crates/legacy-boy
git mv src crates/legacy-boy/src
git mv Cargo.toml crates/legacy-boy/Cargo.toml
```

- [ ] **Step 3: 写根 workspace Cargo.toml**

```toml
# Cargo.toml（仓库根，新建）
[workspace]
resolver = "3"
members = ["crates/*", "spike/*"]

[workspace.package]
edition = "2024"
license = "MIT"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
directories = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 4: 让 legacy-boy 融入 workspace**

编辑 `crates/legacy-boy/Cargo.toml`：包名、bin 名、依赖全部**保持原样**，仅确认 `edition` 与 workspace 一致（若原文件写 `edition = "2024"` 则不动）。文件顶部加注释：

```toml
# 已废弃（2026-07-15 决议）：boy CLI 永不回归。
# 本 crate 仅作 dozerd 的种子代码（config/process/doctor），迁移完成后删除。
# 禁止添加任何新功能。
```

- [ ] **Step 5: 验证 workspace 构建与测试**

Run: `cargo build && cargo test 2>&1 | tail -3 && ls target/debug/boy`
Expected: 构建通过、测试与 Step 1 基线一致、`boy` 二进制仍产出。

- [ ] **Step 6: 重写 CLAUDE.md**

```markdown
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目现实（2026-07-15 起）

本仓库是 **Dozer** —— 站在用户（甲方）一侧的、agent 中立的 AI 治理与验收层（macOS 先发，Rust workspace）。
权威文档：

- 规格（唯一需求真相源）：`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`
- 实现计划系列：`docs/superpowers/plans/2026-07-15-dozer-p1a-*.md` 起
- UI 设计：Figma "Dozer Phase 1 UI"（12 帧）；参考截图 `design/参考/`

## Workspace 布局

| crate | 职责 |
|-------|------|
| `crates/dozer-core` | 共享类型、路径、UDS 协议 |
| `crates/dozerd` | session daemon：PTY 池、会话存活、验收闭环存储（bin: `dozerd`） |
| `crates/dozer-app` | iced 0.14 GUI（bin: `dozer`） |
| `crates/dozer-hook` | 被 agent hooks 调用的零依赖小二进制（bin: `dozer-hook`） |
| `crates/legacy-boy` | **已废弃**的 byteboy v0（bin: `boy`）；仅作 dozerd 种子，禁止扩展 |
| `spike/*` | 一次性技术验证，随时可删 |

## 构建与测试

```bash
cargo build                    # 全 workspace
cargo test -p dozerd           # 单 crate 测试
cargo run -p dozer-app         # 跑 GUI
cargo clippy --all-targets && cargo fmt
```

## 关键裁决（违反即错）

- boy CLI 已废弃，永不回归；agent 启动/模型托管/doctor 全归 dozerd。
- GUI 只用 iced 0.14 生态；预览 WebView 走 wry 子视图叠加，⌘K 打开时隐藏预览。
- mac 先发但架构留门：不引入 Swift/AppKit 专属能力；核心不依赖 Node/Python。
- 一期范围以规格 §3"一期范围裁剪"为准；显式未决项（规格 §8）不得擅自定死。
- 主题 ByteBoy2077：bg `#0a0e16`、金 `#F2D94E`（甲方动作专属）、奶油文字 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "重构：workspace 化，byteboy 移入 crates/legacy-boy 作废弃种子；重写 CLAUDE.md

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: dozer-core 骨架——共享路径模块

**Files:**
- Create: `crates/dozer-core/Cargo.toml`
- Create: `crates/dozer-core/src/lib.rs`
- Create: `crates/dozer-core/src/paths.rs`

**Interfaces:**
- Consumes: workspace 依赖 `directories`、`anyhow`。
- Produces: `dozer_core::paths::{config_dir() -> PathBuf, state_dir() -> PathBuf, socket_path() -> PathBuf}`——P1b 的 dozerd 与 P1c 的 app 都依赖这三个签名。

- [ ] **Step 1: 建 crate**

```toml
# crates/dozer-core/Cargo.toml
[package]
name = "dozer-core"
version = "0.1.0"
edition.workspace = true

[dependencies]
anyhow.workspace = true
directories.workspace = true
serde.workspace = true
```

- [ ] **Step 2: 写失败测试**

```rust
// crates/dozer-core/src/paths.rs
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_dozer() {
        assert!(config_dir().ends_with("dozer"));
    }

    #[test]
    fn state_dir_ends_with_dozer() {
        assert!(state_dir().ends_with("dozer"));
    }

    #[test]
    fn socket_path_is_under_state_dir() {
        let s = socket_path();
        assert!(s.starts_with(state_dir()));
        assert_eq!(s.file_name().unwrap(), "dozerd.sock");
    }
}
```

`crates/dozer-core/src/lib.rs`：

```rust
pub mod paths;
```

Run: `cargo test -p dozer-core`
Expected: FAIL —— `config_dir` 等未定义。

- [ ] **Step 3: 最小实现**

在 `paths.rs` 测试模块上方补：

```rust
use directories::ProjectDirs;

fn dirs() -> ProjectDirs {
    ProjectDirs::from("ai", "byteboy", "dozer").expect("home directory must exist")
}

/// ~/Library/Application Support 或 XDG config 下的 dozer 配置目录
pub fn config_dir() -> PathBuf {
    dirs().config_dir().to_path_buf()
}

/// 运行态（PID、socket、滚屏缓存）目录
pub fn state_dir() -> PathBuf {
    dirs()
        .state_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dirs().data_local_dir().to_path_buf())
}

/// dozerd 的 Unix Domain Socket 路径
pub fn socket_path() -> PathBuf {
    state_dir().join("dozerd.sock")
}
```

注意：macOS 上 `ProjectDirs::state_dir()` 返回 `None`，实现里已用 `data_local_dir` 兜底——`config_dir_ends_with_dozer` 在 mac 上以目录名结尾 `dozer` 断言（`.../Application Support/ai.byteboy.dozer` 会失败）。若失败，把三个测试断言改为 `path.to_string_lossy().contains("dozer")`，这是平台命名差异，不是缺陷。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-core`
Expected: PASS（3 个测试）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core
git commit -m "feat(core): dozer-core 骨架——config/state/socket 路径

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: dozerd / dozer-app / dozer-hook 三个可运行骨架

**Files:**
- Create: `crates/dozerd/Cargo.toml`, `crates/dozerd/src/main.rs`
- Create: `crates/dozer-hook/Cargo.toml`, `crates/dozer-hook/src/main.rs`, `crates/dozer-hook/src/payload.rs`
- Create: `crates/dozer-app/Cargo.toml`, `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `dozer_core::paths::state_dir()`。
- Produces: 三个可执行文件 `dozerd` / `dozer` / `dozer-hook`；`dozer_hook::payload::HookPayload { event: String, ts_ms: u64, data: serde_json::Value }`（P1f 的事件信封雏形，JSON 单行输出）。

- [ ] **Step 1: dozerd 骨架**

```toml
# crates/dozerd/Cargo.toml
[package]
name = "dozerd"
version = "0.1.0"
edition.workspace = true

[dependencies]
dozer-core = { path = "../dozer-core" }
anyhow.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

```rust
// crates/dozerd/src/main.rs
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let state = dozer_core::paths::state_dir();
    std::fs::create_dir_all(&state)?;
    tracing::info!(state_dir = %state.display(), "dozerd 骨架启动（P1b 实现会话内核）");
    Ok(())
}
```

Run: `cargo run -p dozerd`
Expected: 打印 info 日志（含 state_dir 路径）后正常退出，state 目录已创建。

- [ ] **Step 2: dozer-hook 骨架（payload 先行 + 失败测试）**

```toml
# crates/dozer-hook/Cargo.toml
[package]
name = "dozer-hook"
version = "0.1.0"
edition.workspace = true

[dependencies]
# 刻意零重依赖：启动速度是 hook 小工具的生命线（kooky 同款拆法）
serde.workspace = true
serde_json.workspace = true
```

```rust
// crates/dozer-hook/src/payload.rs
use serde::{Deserialize, Serialize};

/// dozer-hook 发往 dozerd 的事件信封（P1f 扩展 data 语义）
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct HookPayload {
    pub event: String,
    pub ts_ms: u64,
    pub data: serde_json::Value,
}

impl HookPayload {
    pub fn new(event: &str, data: serde_json::Value) -> Self {
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before epoch")
            .as_millis() as u64;
        Self { event: event.to_string(), ts_ms, data }
    }

    /// 单行 JSON（JSON Lines 协议帧）
    pub fn to_json_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("payload serializes");
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_line_is_single_line_and_roundtrips() {
        let p = HookPayload::new("turn_completed", serde_json::json!({"session": "s1"}));
        let line = p.to_json_line();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: HookPayload = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(back, p);
    }
}
```

```rust
// crates/dozer-hook/src/main.rs
mod payload;
use payload::HookPayload;
use std::io::Read;

fn main() {
    // 用法：dozer-hook <event-name>；stdin 为 agent hooks 传入的 JSON（可为空）
    let event = std::env::args().nth(1).unwrap_or_else(|| "unknown".into());
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let data = serde_json::from_str(&stdin).unwrap_or(serde_json::Value::Null);
    // P1f 前先打到 stdout；P1f 改为写 dozerd 的 UDS
    print!("{}", HookPayload::new(&event, data).to_json_line());
}
```

Run: `cargo test -p dozer-hook`
Expected: PASS。
Run: `echo '{"tool":"Bash"}' | cargo run -q -p dozer-hook -- tool_used`
Expected: 单行 JSON，含 `"event":"tool_used"` 与 `"tool":"Bash"`。

- [ ] **Step 3: dozer-app iced 0.14 hello 窗口**

```toml
# crates/dozer-app/Cargo.toml
[package]
name = "dozer-app"
version = "0.1.0"
edition.workspace = true

[[bin]]
name = "dozer"
path = "src/main.rs"

[dependencies]
dozer-core = { path = "../dozer-core" }
iced = { version = "0.14", features = ["tokio"] }
```

```rust
// crates/dozer-app/src/main.rs
use iced::widget::{container, text};
use iced::{Color, Element, Theme};

// ByteBoy2077 tokens（P1c 提炼进 theme 模块）
const BG: Color = Color::from_rgb(0.039, 0.055, 0.086); // #0a0e16
const CREAM: Color = Color::from_rgb(1.0, 0.898, 0.706); // #FFE5B4

#[derive(Default)]
struct App;

#[derive(Debug, Clone)]
enum Message {}

fn update(_state: &mut App, _message: Message) {}

fn view(_state: &App) -> Element<'_, Message> {
    container(text("Dozer — P1a 骨架").size(24).color(CREAM))
        .center_x(iced::Fill)
        .center_y(iced::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(BG.into()),
            ..container::Style::default()
        })
        .into()
}

fn main() -> iced::Result {
    iced::application("Dozer", update, view).run()
}
```

Run: `cargo run -p dozer-app`
Expected: 深海军蓝窗口 + 奶油色标题文字；关窗正常退出。若 iced 0.14 对 `iced::application` 的 builder 签名有漂移，以 docs.rs/iced/0.14 首页示例对齐（验收标准不变：`cargo run -p dozer-app` 出深色窗口）。

- [ ] **Step 4: 全量验证 + Commit**

Run: `cargo build && cargo test && cargo clippy --all-targets 2>&1 | tail -3`
Expected: 全绿（clippy 无 error；warning 顺手清零）。

```bash
git add crates/dozerd crates/dozer-hook crates/dozer-app
git commit -m "feat: dozerd/dozer/dozer-hook 三个可运行骨架（HookPayload JSON Lines 雏形）

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Spike A —— winit + wry 子视图叠加（先证 wry 路径）

**Files:**
- Create: `spike/webview-child/Cargo.toml`
- Create: `spike/webview-child/src/main.rs`

**Interfaces:**
- Consumes: 无（独立验证程序）。
- Produces: spike 结论输入 Task 6 报告；不产出被依赖的代码。

- [ ] **Step 1: 建 spike crate**

```toml
# spike/webview-child/Cargo.toml
[package]
name = "spike-webview-child"
version = "0.1.0"
edition.workspace = true
publish = false

[dependencies]
winit = "0.30"
wry = "0.48"   # 以 cargo add wry 拉到的最新版为准；须支持 build_as_child
```

- [ ] **Step 2: 写验证程序**

```rust
// spike/webview-child/src/main.rs
//! Spike A：验证 wry WebView 作为 winit 窗口子视图叠加。
//! 验收：右半区渲染网页；窗口 resize 时 bounds 跟随；按 `h` 隐藏/显示 webview；
//!       webview 内滚动、点击、文本框输入（含中文 IME）正常。
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebView, WebViewBuilder};

const PREVIEW_URL: &str = "https://byteboy.ai";

#[derive(Default)]
struct App {
    window: Option<Window>,
    webview: Option<WebView>,
    visible: bool,
}

fn right_half(size: winit::dpi::PhysicalSize<u32>, scale: f64) -> Rect {
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    Rect {
        position: LogicalPosition::new(w / 2.0, 0.0).into(),
        size: LogicalSize::new(w / 2.0, h).into(),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let window = el
            .create_window(
                Window::default_attributes()
                    .with_title("Spike A: wry child over winit")
                    .with_inner_size(LogicalSize::new(1200.0, 800.0)),
            )
            .expect("create window");
        let bounds = right_half(window.inner_size(), window.scale_factor());
        let webview = WebViewBuilder::new()
            .with_url(PREVIEW_URL)
            .with_bounds(bounds)
            .build_as_child(&window)
            .expect("build child webview");
        self.window = Some(window);
        self.webview = Some(webview);
        self.visible = true;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(wv)) = (&self.window, &self.webview) {
                    let _ = wv.set_bounds(right_half(size, w.scale_factor()));
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent { logical_key: Key::Character(c), state: ElementState::Pressed, .. },
                ..
            } if c == "h" => {
                // 模拟"⌘K 打开命令面板 → 隐藏预览"
                if let Some(wv) = &self.webview {
                    self.visible = !self.visible;
                    let _ = wv.set_visible(self.visible);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::default()).expect("run app");
}
```

- [ ] **Step 3: 运行并逐项人工验收**

Run: `cargo run -p spike-webview-child`
逐项确认（每项在 Task 6 报告中记 ✓/✗ + 备注）：
1. 右半区渲染 byteboy.ai，左半区为窗口底色；
2. 拖拽窗口尺寸，webview 始终占右半、无残影/闪烁；
3. 按 `h` 两次：隐藏 ↔ 显示，延迟无感；
4. webview 内滚动与链接点击正常；
5. 地址栏若有输入框：中文 IME 输入正常；
6. 焦点在左半区（点击左侧）时按键不进 webview。

Expected: 6 项全 ✓（wry 官方支持该模式，预期通过）。任何 ✗ 记录现象与系统日志。

- [ ] **Step 4: Commit**

```bash
git add spike/webview-child
git commit -m "spike(A)：winit+wry 子视图叠加验证程序

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Spike B —— iced_winit 集成 shell + wry 子视图（生产路径）

**Files:**
- Create: `spike/iced-webview/Cargo.toml`
- Create: `spike/iced-webview/src/main.rs`

**Interfaces:**
- Consumes: Spike A 证实的 wry API 用法（`build_as_child`/`set_bounds`/`set_visible`）。
- Produces: 一期 GUI 架构裁决（Task 6 报告）：dozer-app 采用「iced_winit 自持 event loop + iced_wgpu 渲染 + wry 子视图」的集成形态。

- [ ] **Step 1: 建 crate**

```toml
# spike/iced-webview/Cargo.toml
[package]
name = "spike-iced-webview"
version = "0.1.0"
edition.workspace = true
publish = false

[dependencies]
iced_winit = "0.14"
iced_wgpu  = "0.14"
iced_widget = "0.14"
winit = "0.30"
wry = "0.48"
```

（版本以 `cargo add iced_winit iced_wgpu iced_widget` 实际解析为准，三者必须同一 iced 世代。）

- [ ] **Step 2: 以官方 integration 示例为底座**

从 iced 仓库 tag `0.14.0` 复制 `examples/integration/src/main.rs` 作为本 crate `src/main.rs` 起点（该示例演示：自持 winit event loop + iced_wgpu 渲染 iced 控件到自己的 wgpu surface——正是 dozer-app 的生产形态）。

Run: `cargo run -p spike-iced-webview`
Expected: 官方示例原样跑通（wgpu 背景 + iced 控件）。先保证底座绿，再动刀。

- [ ] **Step 3: 叠加 wry 子视图 + 布局联动 + 隐藏切换**

在示例基础上做四处改动（示例代码结构以 0.14 实际为准，改动点语义如下，每处都有明确锚点）：

```rust
// 1) 窗口创建后（拿到 Arc<winit::window::Window> 处）追加：
let webview = wry::WebViewBuilder::new()
    .with_url("https://byteboy.ai")
    .with_bounds(preview_bounds(window.inner_size(), window.scale_factor()))
    .build_as_child(window.as_ref())
    .expect("child webview over iced window");
let mut preview_visible = true;

// 2) 顶部（与示例的 controls 并列）加一个 iced 按钮，消息命名：
#[derive(Debug, Clone)]
enum Message {
    ToggleGate, // 模拟 ⌘K：隐藏/显示预览
    // ……示例原有消息保留
}

// 3) update 分支：
Message::ToggleGate => {
    preview_visible = !preview_visible;
    let _ = webview.set_visible(preview_visible);
}

// 4) WindowEvent::Resized 分支追加：
let _ = webview.set_bounds(preview_bounds(new_size, window.scale_factor()));

// 布局函数：预览占窗口右侧 40%（模拟左二 pane 的矩形区域）
fn preview_bounds(size: winit::dpi::PhysicalSize<u32>, scale: f64) -> wry::Rect {
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(w * 0.35, 40.0).into(),
        size: wry::dpi::LogicalSize::new(w * 0.40, h - 40.0).into(),
    }
}
```

- [ ] **Step 4: 运行并逐项人工验收**

Run: `cargo run -p spike-iced-webview`
逐项确认（记入 Task 6 报告）：
1. iced 控件（按钮等）与 webview 同窗共存，各画各的区域；
2. 点 iced 按钮 → webview 隐藏，再点恢复（⌘K 语义成立）；
3. resize 时 webview 跟随 `preview_bounds`，iced 布局不与之打架；
4. webview 隐藏期间，其占位区域可见 iced/wgpu 背景（证明 z-order 关系符合预期：webview 恒在上，隐藏即让位）；
5. 点击 iced 区 → 键盘事件进 iced；点击 webview 区 → 滚动/输入进 webview（焦点切换无死区）；
6. CPU/GPU 占用无异常（Activity Monitor 目测，空闲 <10% CPU）。

Expected: 全 ✓ ⇒ 生产路径成立。若 4/5 出现焦点或合成异常 ⇒ 记录细节，并测试降级路径（`set_bounds` 挪到屏外代替 `set_visible`）后再判。

- [ ] **Step 5: Commit**

```bash
git add spike/iced-webview
git commit -m "spike(B)：iced_winit 集成 shell 叠加 wry 子视图——生产路径验证

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Spike 报告与 Go/No-Go 裁决

**Files:**
- Create: `docs/superpowers/specs/2026-07-15-spike-report-webview.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（§4 风险条目回填结论，一行）

**Interfaces:**
- Consumes: Task 4/5 的逐项验收记录。
- Produces: P1c/P1d 的架构前提（集成形态 + 已知约束清单）。

- [ ] **Step 1: 写报告**

```markdown
# Spike 报告：iced × wry WebView 合成（P1a Task 4/5）

日期：<执行日> · 环境：macOS <版本> / Apple Silicon / iced 0.14.x / wry <版本> / winit 0.30.x

## Spike A（winit+wry 子视图）
| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | 子视图区域渲染 | ✓/✗ | |
| 2 | resize 跟随 | ✓/✗ | |
| 3 | 隐藏/显示 | ✓/✗ | |
| 4 | 滚动/点击 | ✓/✗ | |
| 5 | 中文 IME | ✓/✗ | |
| 6 | 焦点隔离 | ✓/✗ | |

## Spike B（iced_winit 集成 + wry）
| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1–6 | （同 Task 5 清单） | | |

## 裁决
- [ ] **GO**：dozer-app 采用 iced_winit 自持 event loop + iced_wgpu + wry 子视图；⌘K 隐藏预览方案成立。
- [ ] **降级**：<触发的验收项与现象>；改用 <set_bounds 屏外 / 独立预览窗>。

## 移交 P1c/P1d 的约束
- <焦点/滚动/缩放的已知怪癖清单，逐条>
```

（报告必须填实测结果，不得留 ✓/✗ 模板原样。）

- [ ] **Step 2: 规格回填一行**

在规格 §4「WebView 合成（风险已降级）」条目末尾追加：
`spike 结论（2026-07-<日>）：<GO/降级>，详见 specs/2026-07-15-spike-report-webview.md`。

- [ ] **Step 3: 全量回归 + Commit**

Run: `cargo build && cargo test 2>&1 | tail -3`
Expected: 全绿。

```bash
git add docs/superpowers/specs
git commit -m "spike：合成验证报告与 Go/No-Go 裁决，规格回填结论

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **规格覆盖**：本计划只承诺 P1a 范围（workspace + spike）；规格 §9 顺序①与仓库结构决议全覆盖；§9 ②-⑦ 由 P1b–P1g 承接（见系列表）。
2. **占位符扫描**：Task 5 Step 3 以"官方示例为底座 + 四处锚点改动"表达，代码语义完整、锚点明确，属 spike 的合理形态；其余任务代码齐全。
3. **类型一致性**：`dozer_core::paths::{config_dir,state_dir,socket_path}`、`HookPayload{event,ts_ms,data}`、二进制名 `dozerd/dozer/dozer-hook` 在各任务与 CLAUDE.md 中一致。
