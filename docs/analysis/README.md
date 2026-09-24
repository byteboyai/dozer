# 竞品/同类项目分析

对 AI 开发环境类产品的代码级分析,用于指导 ByteBoy 的架构决策。

| 文档 | 项目 | 一句话 |
|------|------|--------|
| [dozer-v2架构分析.md](../dozer-v2/dozer-v2架构分析.md) | Dozer V2 产品与架构 | 微内核 Host、插件 App、MCP 能力网络与 Vibe Coding 闭环 |
| [dozer-v2开源生态调研.md](../dozer-v2/dozer-v2开源生态调研.md) | Dozer V2 开源生态 | 同类项目、局部参考、依赖候选与工程验证路线 |
| [dozer-v2-逐仓代码复核.md](./dozer-v2-逐仓代码复核.md) | Dozer V2 逐仓代码复核 | 59 个仓库的源码证据、V2 价值与去留裁决 |

## 两个项目共同验证的"标准件"

- OSC 7(cwd)/ OSC 133(命令退出码)shell 集成
- hook 端点 + surface/pane 环境变量路由(无状态 hook 小工具,配对逻辑在主进程)
- agent 会话 resume id 持久化(`--resume <id>`)
- git worktree 管理(一 agent 一 worktree)
- agent 用量/限额监控

## 对 ByteBoy 的核心结论

v0 CLI → v1 Rust session daemon(PTY 持有 + hook socket)→ 前端只是 daemon 的客户端。
"会话存活"(daemon 持有 PTY)必须在最底层设计时定下来,后补等于重写。
