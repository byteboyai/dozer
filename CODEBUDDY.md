# CODEBUDDY.md

This file provides guidance to CodeBuddy Code when working with code in this repository.

## 项目现实

本仓库是 **Dozer** —— 站在用户（甲方）一侧的、agent 中立的 AI 治理与验收层（macOS 先发，Rust workspace，edition 2024）。它不是 IDE / 编辑器 / agent，而是管理"目标定义 → agent 派活 → 交付验收 → 史料沉淀"的完整闭环。

权威文档（按优先级）：
- 规格（唯一需求真相源）：`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`
- 实现计划系列：`docs/superpowers/plans/`（每个 P1x 阶段一份 spec + plan + acceptance 三件套）
- UI 设计：Figma "Dozer Phase 1 UI"；参考截图 `design/参考/`
- 竞品分析：`docs/analysis/`

## 构建与测试

```bash
cargo build                    # 全 workspace（含 crates/* 与 spike/*）
cargo test                     # 全 workspace 测试
cargo test -p dozerd           # 单 crate 测试
cargo test -p dozerd session_survival   # 单测试文件
cargo test -p dozer-client --test against_real_daemon   # 集成测试（会拉起真实 dozerd）
cargo run -p dozer-app         # 跑 GUI（自动拉起同目录/PATH 上的 dozerd）
cargo run -p dozerd            # 单独跑 daemon
cargo clippy --all-targets && cargo fmt     # 提交前必过

scripts/build-macos-app.sh [debug|release]   # 打 Dozer.app 包（debug 默认 release）
```

注意：`dozer-client` 的 `against_real_daemon` 集成测试会真起 dozerd，跑前确认没有残留 daemon 占着 `state_dir/dozerd.sock`。

## Workspace 布局

| crate | 职责 |
|-------|------|
| `crates/dozer-core` | 共享类型：`paths`（config/state/socket 路径）、`protocol`（UDS 协议帧、AgentState 四态机） |
| `crates/dozerd` | session daemon（bin `dozerd`）：PTY 池、ring buffer 滚屏、会话存活、SQLite 存储（项目 + 验收记录）、hook 事件→状态机 |
| `crates/dozer-client` | 给 GUI 用的异步 UDS 客户端（tokio）；`attach` 走长连接 + `TermEvent` channel |
| `crates/dozer-app` | iced 0.14 GUI（bin `dozer`）：winit 自持 event loop + iced_wgpu + wry 子视图 |
| `crates/dozer-hook` | 被 agent hooks 调用的零依赖小二进制（bin `dozer-hook`）：单向转发 hook 事件给 dozerd |
| `crates/legacy-boy` | **已废弃**的 byteboy v0（bin `boy`）；仅作 dozerd 种子代码，**禁止扩展** |
| `spike/*` | 一次性技术验证，随时可删 |

## 运行时架构（跨多文件才能看清的数据流）

**进程拓扑**：`dozer`（GUI）启动时探测 `state_dir/dozerd.sock`；连不上就 `spawn` 同目录或 PATH 上的 `dozerd`，1s 间隔重试 3 次（`dozer-app/src/main.rs::ensure_daemon`）。`dozer-hook` 由 Claude Code hooks 独立拉起，连同一个 socket。

**UDS 协议**（`dozer-core/src/protocol.rs`）：newline-delimited JSON，`Request`/`Reply` 两个 `#[serde(tag="type")]` 枚举。PTY 字节走 base64（`data_b64`）。`Attach` 是长连接：先回一帧 `Attached{snapshot_b64, next_offset}`，之后持续推 `Output`/`AgentEvent`/`Exited`，消费端断开时 daemon 靠 `lines.next_line()` 的 EOF 退出（`dozerd/src/server.rs::handle_conn`）。客户端在 `dozer-client/src/lib.rs::attach` 里用 `tx.closed()` select 提前结束读任务，避免任务/FD 滞留。

**会话存储**（`dozerd`）：每个 `Session` 持有 `portable_pty` 的 master + `RingBuffer`（`SCROLLBACK_CAP`）+ `tokio::sync::broadcast` 通道；新 attach 先从 ring 拿 snapshot，再订阅 broadcast。`SessionRegistry` 是所有活跃会话的根。

**hook → 状态机**：`dozer-hook` 读 stdin 当 JSON、取 `DOZER_SESSION_ID` 环境变量、`UnixStream` 连 dozerd、200ms 写超时、**发完即走不读应答**，任何错误都静默（绝不拖慢/拖垮 agent）。`dozerd/src/server.rs::agent_state_for` 把 7 个 Claude Code hook 事件映射到 `AgentState` 四态（Idle/Running/AwaitingInput/TurnEnded）；未知事件不改状态。

**持久化**：`state_dir/dozer.db`（rusqlite bundled）由 `dozerd::projects::ProjectStore` 和 `dozerd::acceptance::AcceptanceStore` 共享；验收记录是"甲方资产"，acceptor 字段在 daemon 侧补 `"user"`。

**GUI 渲染**：`dozer-app/src/main.rs` 是手写 `winit::application::ApplicationHandler`，不是 iced 默认的 `Application` trait——因为要同窗叠加 wry 子视图（预览 / 浏览器两个独立 webview 池，按 tab id 索引）。tokio runtime 由 `main` 持有，UI 线程只 `handle.spawn`，启动序列的 `block_on` 是唯一例外。键盘/IME 经 `keymap` 翻字节直接回灌 PTY，回显完全走真实 PTY 回路。`sync_previews` 在每轮事件后做 webview 池差集同步（建/毁/导航/可见性）。

**shell 集成**：`dozer-hook install` 幂等增量合并 `~/.claude/settings.json`，只增删 Dozer 自己的条目（识别靠 command 含 `dozer-hook`/`dozer_hook`），绝不动用户其他配置。注入会话 id 靠 `DOZER_SESSION_ID` 环境变量（`dozerd/src/shell_integration.rs`）。

## 关键裁决（违反即错）

- **boy CLI 已废弃，永不回归**；agent 启动 / 模型托管 / doctor 全归 dozerd。`legacy-boy` 只准删不准扩。
- **GUI 只用 iced 0.14 生态**；预览 WebView 走 wry 子视图叠加，webview 恒在 GPU 内容之上——凡是需要盖住它的原生浮层（如验收 tab 全屏态）都要显式隐藏 webview，不能指望层级自然遮挡。**⌘K 命令面板未实现**（规格 §3 已裁掉一期范围，顶栏曾有的纯视觉占位搜索框已于后续迭代移除，见 `app.rs` 中该处的移除说明注释）；「⌘K 打开时隐藏预览」这条早期设计陈述作废，不要再引用它。
- **wry 统一 0.55.1**，`WebViewBuilder::new().with_url().with_bounds().build_as_child(&window)` 用法已验证。ToggleGate 类副作用必须在拥有 window/webview 句柄的事件环执行，纯视图层拿不到句柄（spike 约束 2）。
- **mac 先发但架构留门**：不引入 Swift/AppKit 专属能力；核心不依赖 Node/Python。
- **一期范围以规格 §3"一期范围裁剪"为准**；显式未决项（规格 §8）不得擅自定死。
- **主题 ByteBoy2077**：bg `#0a0e16`、金 `#F2D94E`（甲方动作专属）、奶油文字 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。配色常量在 `dozer-app/src/theme.rs`。

## 路径约定（`dozer-core/src/paths.rs`）

`directories::ProjectDirs::from("ai","byteboy","dozer")`：
- `config_dir()` → 配置
- `state_dir()` → 运行态（`dozerd.sock`、`dozer.db`、滚屏缓存）
