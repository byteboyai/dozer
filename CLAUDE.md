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
| `crates/dozer-client` | dozer-app/dozer-mcp 共用的 UDS 客户端库(`Client`) |
| `crates/dozer-mcp` | 面向外部 CLI agent 的只读 MCP stdio server（bin: `dozer-mcp`） |
| `spike/*` | 一次性技术验证，随时可删 |

## 构建与测试

```bash
cargo build                    # 全 workspace
cargo test -p dozerd           # 单 crate 测试
cargo run -p dozer-app         # 跑 GUI
cargo clippy --all-targets && cargo fmt
```

## 关键裁决（违反即错）

- boy CLI 已废弃，永不回归；agent 启动/模型托管/doctor 全归 dozerd。`crates/legacy-boy` 已删除（2026-08-12）——删除时 dozerd 尚未实际迁入 `config`/`process`/`doctor` 这三块，旧实现只留在 git 历史（删除前的最后一次提交）里，之后要做这几块时得从那份历史重新参考，不是已经迁完。
- GUI 只用 iced 0.14 生态；预览 WebView 走 wry 子视图叠加，⌘K 打开时隐藏预览。
- mac 先发但架构留门：不引入 Swift/AppKit 专属能力；核心不依赖 Node/Python。
- 一期范围以规格 §3"一期范围裁剪"为准；显式未决项（规格 §8）不得擅自定死。
- 主题 ByteBoy2077：bg `#0a0e16`、金 `#F2D94E`（甲方动作专属）、奶油文字 `#FFE5B4`、青 `#47DEF0`、绿 `#1AD585`。
- 新增/改造 icon 按钮、tab 类 UI 时优先复用统一组件（`icons::icon_button_entry`/`tabs::tab_core`），不要重新手写一套 `MouseArea`+`on_enter`/`on_exit` 接线；确需自定义（形状/交互模式明显不同）要在 plan 里说明理由，不是绝对禁止。
