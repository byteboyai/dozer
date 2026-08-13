# Dozer

站在用户（甲方）一侧的、agent 中立的 **AI 治理与验收层**。macOS（Apple Silicon）先发，Rust workspace。

> 执行由 AI 拉平，判断由 Dozer 拉平。

Dozer 不是 IDE、不是编辑器、不是 AI 聊天应用、不是终端模拟器，也不是又一个 agent。它管理从目标定义、agent 派活、交付验收到演进史沉淀的完整闭环，让 AI 产出**可持续演进的作品**。

## 权威文档

- 规格（唯一需求真相源）：`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`
- 实现计划系列：`docs/superpowers/plans/`
- 开发者指引：`CLAUDE.md`

## Workspace

| crate | 职责 |
|-------|------|
| `crates/dozer-core` | 共享类型、路径、UDS 协议 |
| `crates/dozerd` | session daemon：PTY 池、会话存活、验收闭环存储（bin: `dozerd`） |
| `crates/dozer-app` | iced 0.14 GUI（bin: `dozer`） |
| `crates/dozer-hook` | 被 agent hooks 调用的零依赖小二进制（bin: `dozer-hook`） |
| `crates/dozer-client` | dozer-app/dozer-mcp 共用的 UDS 客户端库（`Client`） |
| `crates/dozer-mcp` | 面向外部 CLI agent 的只读 MCP stdio server（bin: `dozer-mcp`） |
| `spike/*` | 一次性技术验证，随时可删 |

> `boy` CLI（legacy-boy）已于 2026-08-12 删除，永不回归；agent 启动/模型托管/doctor 全归 dozerd。

## 构建

```bash
cargo build                    # 全 workspace
cargo test -p dozerd           # 单 crate 测试
cargo run -p dozer-app         # 跑 GUI
cargo clippy --all-targets && cargo fmt
```
