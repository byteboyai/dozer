# Orca 代码级分析

> 分析对象:https://github.com/stablyai/orca (本地副本 `/Users/chrischiang/AI/orca`)
> 分析日期:2026-07-14 · TypeScript ~39.4 万行 / 6900 文件(近半为测试) / MIT

## 一句话定位

Orca 是 **Electron + TypeScript 的跨平台 AI agent 编排器**:每个 agent 跑在自己的 git worktree 里,统一在一处跟踪。支持 macOS/Windows/Linux + iOS/Android 手机伴侣端。规模是 kooky 的 ~19 倍——公司级产品,不是个人项目。

与 kooky 的本质区别:**kooky 是"更懂 agent 的终端"(深度在渲染和 macOS 原生体验),orca 是"agent 的调度台"(深度在编排、远程和分发)——终端在 orca 里只是一个组件,xterm.js 够用就行。**

## 分层架构

```
src/
├── main/       Electron 主进程,~80 个按领域切分的模块目录
│               (claude/ codex/ cursor/ …30 个 agent 各一目录,
│                pty/ daemon/ ssh/ git/ github/ gitlab/ linear/ jira/
│                browser/ computer/ sqlite/ speech/ emulator/ …)
├── renderer/   React UI(shadcn + design tokens)
├── shared/     ~500 文件的传输无关纯逻辑层 —— 全项目的精华
├── relay/      部署到远程主机/WSL 的独立服务端(fs/git/pty/hook 处理器)
├── cli/        orca CLI(通过 RPC 操控运行中的 app)
├── mobile/     React Native 手机端(独立 pnpm workspace)
└── native/     每 OS 一个的 computer-use 原生模块、macOS 通知等
```

运行时依赖出奇地少(22 个):`node-pty`、`@xterm/headless`、`ssh2`、`ws`、`tweetnacl`(手机端 E2EE)、`zod`、`sherpa-onnx`(本地语音)。重的东西都是自己写的。

注意:`src/main/ghostty/` 不是嵌入 libghostty,只是**解析 ghostty 主题/配置文件格式**用于导入——与 kooky 走了完全相反的终端路线。

## 四个最有含金量的子系统

### 1. PTY 守护进程(`src/main/daemon/`,~90 文件)——技术含量最高

PTY 所有权从 Electron 主进程剥离到**独立后台 daemon**:

- unix socket + token 鉴权,NDJSON/二进制帧协议
- daemon 内跑 `@xterm/headless` 无头终端模拟器维护每个会话的屏幕状态,`addon-serialize` 做快照
- 效果:**app 重启/升级/崩溃时 agent 继续跑,重开后 reattach 恢复现场**——本质上是用 Node 重新实现了 tmux
- 工程深度的痕迹:`session-ingest-throughput.bench.test.ts`(吞吐基准)、`headless-emulator-fidelity.fuzz.test.ts`(模糊测试保真度)、`hibernation-cold-restore-repro.test.ts`、`priority-semaphore.ts`(背压)——这是和 Node 运行时性能搏斗的证据

### 2. 执行宿主抽象 + relay(自研远程开发协议)

- `ExecutionHostId = 'local' | 'ssh:<target>' | 'runtime:<env>'`(后者为临时 VM),外加 WSL
- `src/relay/` 是部署到远端的自包含服务端:fs(带 ripgrep 安装/回退)、git(完整 worktree/staging/diff)、pty、agent-hook 四类 handler——相当于自研小号 VS Code Server
- AGENTS.md 铁律:"所有改动必须考虑 SSH 场景"
- `GitCapabilityCache` 按宿主隔离做能力探测 + 降级(Git 2.25 基线),因为 native/WSL/SSH 三处 git 版本可能都不同

### 3. Agent 状态感知——kooky 的加强混战版

- `shared/agent-hook-listener.ts` 单文件 3987 行,**传输无关**的 hook 监听管线(HTTP 端点,1MB 大小上限、slowloris 防护),同一份代码在本地主进程和远程 relay 各跑一份("relay 归一化,Orca 路由")
- hook 只覆盖 Claude 这类有 hook 系统的 agent;orca 支持 30 个 agent,大部分没有 hook,所以是**多信号融合**:OSC 标题提取、ANSI 输出流扫描(`agent-tui-ansi-fuzz-stream`)、进程表扫描识别(`agent-process-recognition`)、agent 会话文件解析(`grok-session-paths`、Claude subagent roster)
- 外加每家的用量/限额跟踪(`claude-usage`、`codex-usage`、`rate-limits`)和多账号热切换(`claude-accounts`、`codex-accounts`)

### 4. App 本身可被 agent 编程

`orca` CLI 通过 RPC 操控运行中的 app:`orca worktree create`、`snapshot`、`click`、`fill`(内嵌浏览器自动化)、computer use。**agent 可以驱动 orca 自己**完成"开 worktree → 跑另一个 agent → 截图验证"的闭环——产品设计上最超前的一点。

## 工程文化观察

- **shared/ 纯函数化**:几乎每个 `.ts` 配同名 `.test.ts`,状态显式建模为可注入结构(如 `HookListenerState`),39 万行里近半测试还能 daily ship
- **AGENTS.md 是写给 AI 的规范**(项目明显大量由 agent 开发):禁止 `utils/helpers` 命名、禁止解除 max-lines、注释只写"为什么";repro 测试带 issue 号(`repro-7329-remote-snapshot-corruption.test.ts`)
- 工具链全是 Rust 的:oxlint、oxfmt

## kooky vs orca 对照

| | kooky | orca |
|---|---|---|
| 形态 | 原生 macOS 终端 app | Electron 跨平台编排器 |
| 终端 | libghostty(GPU) | node-pty + xterm.js + 自研 PTY daemon |
| 会话存活 | 不存活,重启靠 `--resume` | **daemon 持有 PTY,app 重启无感** |
| agent 状态 | socket hook + OSC 双通道,4 个 agent 精做 | 多信号融合,30 个 agent 广撒网 |
| 远程 | OSC 带内标记(轻) | relay 协议服务端(重,自研远程开发协议) |
| 组织单位 | 目录/tab | **git worktree**(一 agent 一 worktree) |

两个项目独立收敛到了同一套地基:OSC 7/OSC 133 shell 集成、hook 端点 + surface/pane 路由、resume id 持久化、worktree 管理、用量监控。**这套就是此品类的标准件,用什么语言实现都绕不开。**

## 对 Rust 实现的启示

1. **最值得用 Rust 做的,就是 orca 用 Node 做得最吃力的那块:PTY daemon。** 无头终端模拟(`alacritty_terminal` 是现成的无头 VT 状态机)+ 会话快照 + 多客户端 attach——orca 为吞吐和保真写基准/模糊测试对抗 Node 开销,而这正是 Rust 的主场(Zellij 已验证)。一个 Rust session daemon 可同时服务 GUI、TUI、CLI、手机 relay 多种前端
2. **relay 是第二个 Rust 甜点**:orca 必须往远程主机分发 Node bundle(还要处理 `daemon-bundle-staleness` 这类问题);musl 静态链接的单二进制 relay 在分发上是碾压性优势
3. **架构上抄"传输无关核心 + 薄宿主"**:`shared/` 对应 Rust workspace 的核心 crate(协议 + 纯逻辑),daemon/cli/gui/relay 都是薄壳——与 byteboy 设计文档的小 crate 原则同构
4. **不要抄它的广度**:39 万行大量是 30 个 agent 适配、5 个 git 提供商、Linear/Jira、手机端、内嵌浏览器——靠团队每天堆出来的面积。单人/小团队应选 kooky 的深度路线,但把 **"会话存活"这一个 orca 独有的架构点(daemon)纳入 v0 设计**,因为它必须在最底层定下来,后补等于重写

## 对 byteboy 的演进路径

**v0 CLI(agent/model/doctor/config,已规划)→ v1 加 Rust session daemon(PTY 持有 + hook socket,即 orca 的 daemon + kooky 的 HookServer 合体)→ 前端(TUI 或 GUI)只是 daemon 的客户端。** 每一层独立可测,符合设计文档 trait-first、为 v1 留接口的原则。