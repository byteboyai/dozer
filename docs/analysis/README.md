# 竞品/同类项目分析

对 AI 开发环境类产品的代码级分析,用于指导 ByteBoy 的架构决策。

| 文档 | 项目 | 一句话 |
|------|------|--------|
| [kooky-分析.md](./kooky-分析.md) | [iAmCorey/kooky](https://github.com/iAmCorey/kooky) — Swift/macOS,libghostty 上的 AI 终端 | "更懂 agent 的终端",深度在渲染与原生体验 |
| [orca-分析.md](./orca-分析.md) | [stablyai/orca](https://github.com/stablyai/orca) — Electron 跨平台 agent 编排器 | "agent 的调度台",深度在编排、远程与分发 |

## 两个项目共同验证的"标准件"

- OSC 7(cwd)/ OSC 133(命令退出码)shell 集成
- hook 端点 + surface/pane 环境变量路由(无状态 hook 小工具,配对逻辑在主进程)
- agent 会话 resume id 持久化(`--resume <id>`)
- git worktree 管理(一 agent 一 worktree)
- agent 用量/限额监控

## 对 ByteBoy 的核心结论

v0 CLI → v1 Rust session daemon(PTY 持有 + hook socket)→ 前端只是 daemon 的客户端。
"会话存活"(daemon 持有 PTY)必须在最底层设计时定下来,后补等于重写。