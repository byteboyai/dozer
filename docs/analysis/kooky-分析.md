# Kooky 代码级分析

> 分析对象:https://github.com/iAmCorey/kooky (本地副本 `/Users/chrischiang/AI/kooky`)
> 分析日期:2026-07-14 · 版本 v0.35.0 · Swift 21K 行 / 63 文件 / 502 测试 / MIT

## 一句话定位

Kooky 是 **libghostty(Ghostty 终端的 C 内核)之上的一层 AI 工作流管理壳**,不是从零写的终端模拟器。真正的终端能力(VT 解析、GPU/Metal 渲染、PTY、滚动回看、搜索)全部来自 `Vendor/GhosttyKit.xcframework` 二进制依赖;自己的 2 万行代码全花在:会话/窗格模型、agent 状态感知、shell 集成、持久化和 SwiftUI 界面。

## 工程结构(Package.swift 四个 target)

| Target | 作用 | 关键点 |
|--------|------|--------|
| `Kooky` | 只有 main.swift 的薄可执行文件 | 逻辑全在 KookyKit(SPM 不允许测试导入 executable) |
| `KookyKit` | 全部应用逻辑 | 链接 GhosttyKit + Metal 等系统框架 |
| `KookyHook` | 独立小 CLI,被 agent 的 hook 调用 | **刻意不链接 KookyKit**,保持启动快、零依赖 |
| `KookyHookKit` | hook 的 payload 构造/解析 | 抽出来是为了不起子进程就能单测 |

"厚 lib + 薄 exe + 独立 hook 小工具"的拆分本身就值得借鉴。

## 核心机制一:终端引擎抽象(`Terminal/TerminalEngine.swift`)

`TerminalEngine` 是 protocol,`LibghosttyEngine` 是唯一实现。接口全是**回调驱动的语义事件**,不是原始字节流:

- `onPwdChange` — shell 发 OSC 7 时触发(cwd 跟踪的唯一来源)
- `onCommandFinished(exitCode, duration)` — OSC 133;D(shell integration 协议)
- `onTitleChange` — OSC 0/2,显示 ssh 远端标题,也被用作**远程 agent 状态的带内信道**
- `onUserInput`、`onFocus`、搜索生命周期;`sendInput`/`paste` 分离(paste 走 bracketed-paste)

工程化细节:`beginSizePropagationSuspension()` 是**引用计数的**——窗格缩放动画期间挂起 resize 传播,否则一次动画触发 12–24 次 SIGWINCH 会把 conda 用户的 scrollback 冲掉。终端嵌入的坑主要在 resize/焦点/剪贴板,不在渲染。

## 核心机制二:Agent 状态感知(全项目最有含金量的设计)

双通道设计:

### 通道 1:unix socket + hook 小工具(本地 agent)

1. kooky 启动每个 tab 时注入环境变量 `KOOKY_SURFACE_ID=<session UUID>`
2. kooky 给 Claude Code 写一份 `--settings` hooks 配置,让 Claude 在 Stop/UserPromptSubmit/Notification/PreToolUse/PostToolUse 等事件时调用 `kooky-hook <agent> <event>`
3. `kooky-hook` 读环境变量里的 surface id,连 `~/Library/Application Support/kooky/socket`,**写一行 JSON 就退出**(fork-per-event,完全无状态)
4. 主 app 的 `HookServer` 用 DispatchSource accept,单次 read 4KiB,解析后按 UUID 路由到对应 Session

无状态 hook 意味着**配对逻辑全在 app 侧**:`Session.recordToolCallEnd` 优先用 `tool_use_id` 匹配 Pre/Post(并发同名工具调用只有这个是正确身份),回退到 (toolName, identifier);另有 5 秒一次的 orphan 扫描,60 秒没等到 Post 的调用标记为 `stalled`(Claude 崩了/断网)。

退出码契约:0 = 成功或"不在 kooky 里跑,别重试";1 = IPC 失败,shell 侧不推进去重缓存,下个 prompt 重试。

### 通道 2:OSC 标题标记(SSH 远端)

远端机器上没法连本地 socket,让远端 shell 发特制 OSC 标题序列,kooky 在本地终端字节流里识别,设置 `transientAgent`/`remoteHost`。带内信道有污染风险(`claude -p > out` 重定向时不能吐 OSC 字节),所以有 tty 检测门控。

### 会话恢复

Claude 的 hook JSON 带 `session_id`,kooky 持久化为 `conversationId`,下次启动拼 `--resume <id>` 无缝续聊。Agent 定义(`AgentTemplate`)是纯数据:`initialCommand`、`promptLaunchFlag`(如 Copilot 的 `-p`)、`resumeFlag`(Claude 的 `--resume`)、`reportsToolCalls`——加一个 agent 就是加一条记录。

## 核心机制三:Shell 集成(1853 行,坑最多的模块)

需求:shell 在 cd 时发 OSC 7、命令结束发 OSC 133、启动时自动执行 agent 命令但退出后留下干净的 shell。三种 shell 三套方案:

- **zsh**:`ZDOTDIR` 劫持——指向 kooky 的临时目录,里面的 `.zshrc` 先恢复用户原始 ZDOTDIR、source 用户真实的 `.zshenv/.zprofile/.zshrc`,再装 `chpwd`/`precmd` 钩子,最后 `eval $KOOKY_AGENT` 内联启动 agent
- **bash**:launcher 脚本 re-exec(libghostty 强制 login shell,`--rcfile` 语义会被剥掉)
- **fish**:往 `XDG_DATA_DIRS` 前插目录,靠 `vendor_conf.d` 自动加载——不用 `-C` 是因为 Fig/Amazon Q 这类包壳工具会吞掉后者

**shell 集成是此类产品真正的工程成本所在,且方案与实现语言完全无关**——用 Rust 重写时这套脚本可以原样移植。

## 数据模型与持久化

```
窗口 → WorkspaceStore(1730 行,中枢/所有事件的汇聚点)
  └─ Workspace(一个项目目录,可以是 git worktree 或 SSH 远端)
      └─ PaneNode 二叉分割树(.pane 叶子 | .split(方向, 左, 右, 比例))
          └─ Pane → tabs: [Session]
              └─ Session(engine + agent + cwd + git 状态 + 工具调用事件流…)
```

- 持久化(`Persistence.swift`)只存元数据到 `state.json`:PTY 状态无法跨进程存活,重启时按存的 cwd 重新 spawn 引擎、拼 `--resume`
- 运行时字段(activityState、toolCallEvents、搜索状态)**从 Codable 结构里整体缺席**,靠类型系统而非运行时判断保证不落盘
- 新增字段一律 Optional 以兼容旧 state 文件;"从磁盘读的值要 clamp,不能信任"
- git 状态是 **shell 出去调 git CLI**:读路径 1 秒超时 + 丢弃 stderr,写路径(worktree)完整错误透传——同一件事两种容错策略
- 前台进程环境(sysinfo 式)只做兜底,真值来自 prompt hook 上报(`nvm use`/`activate` 改的是 shell 内存,内核 proc env 快照是过期的)

## 用 Rust 实现类似项目的考量

### 第 0 个决策:产品形态

- **路线 A:TUI 复用器(推荐起点)**——跑在用户现有终端里,像 Zellij。`ratatui` + `portable-pty` + VT 解析器。核心价值(agent 状态感知、workspace/worktree、hook 集成)一分不丢,砍掉 GPU 渲染、窗口管理、剪贴板/IME 这些 Rust 生态最痛的部分
- **路线 B:原生 GUI 终端**——FFI 嵌 libghostty(C ABI,Rust 调用和 Swift 一样可行),或 `alacritty_terminal` + `wgpu` 自绘(Zed 的做法)。工作量是路线 A 的 3–5 倍
- **路线 C:Tauri + xterm.js**——最快出 demo,但 GPU 加速/低延迟的卖点没了,不建议

### 子系统 → Rust 映射

| kooky 子系统 | Rust 对应 | 备注 |
|---|---|---|
| libghostty 嵌入 | FFI(bindgen)或 `alacritty_terminal`/`termwiz` | C ABI 回调 → `Box<dyn Fn>` + userdata |
| PTY spawn | `portable-pty`(wezterm 出品) | 跨平台,含 Windows ConPTY |
| `TerminalEngine` protocol | trait + 事件 enum + mpsc channel | `enum EngineEvent { PwdChange(String), … }` 比逐个回调字段更惯用 |
| PaneNode 二叉树 | `enum PaneContent { Pane(Pane), Split{ … Box<PaneNode> … } }` | Swift 的 `indirect case` 就是在模仿 Rust enum |
| HookServer unix socket | `tokio::net::UnixListener` + serde_json 按行 | 几十行的事 |
| KookyHook CLI | 独立 bin,**只用 std** | 启动速度是硬指标(每个 hook 事件 fork 一次);保留 0/1 退出码契约 |
| @Observable UI 响应 | 无等价物:state 变更 → 事件 → 重绘 | 从 Swift 迁移最大的范式差异 |
| @MainActor 单线程 | tokio 单 runtime + actor 风格(状态归一个 task,channel 通信) | 别用 `Arc<Mutex<World>>` |
| Persistence | serde_json + `#[serde(default)]` | 运行时字段不派生 Serialize 即天然不落盘 |
| git 状态/worktree | **照抄:shell 出去调 git**,`tokio::process` + timeout | 别上 git2/gix,status 语义对齐 CLI 很难 |
| 目录监控 | `notify` crate | fsevents 后端现成 |
| 前台进程信息 | `sysinfo` / `libproc` | |
| shell 集成脚本 | **原样移植** | 与宿主语言无关,直接省掉最大一块试错成本 |

### 值得原样继承的设计决策

1. **hook 小工具无状态、配对逻辑在主进程**——fork-per-event 天然崩溃安全,orphan 扫描(60s 判 stalled)状态机照搬
2. **`KOOKY_SURFACE_ID` 环境变量路由**——一个 env var 解决"哪个 tab 的 agent 在说话",多 tab 多 agent 不串线
3. **AgentTemplate 纯数据化**——支持新 agent = 加一条配置记录(与 byteboy 的 `[agents]` TOML 理念一致)
4. **双信道**——本地 socket(可靠、结构化)+ 远程 OSC 带内标记(唯一穿透 ssh 的通道),重定向场景做 tty 门控
5. **运行时状态与持久化状态用类型区分**,新字段一律可缺省

### 工作量判断

21K 行、502 测试、93 个 release 中,"能跑的 demo"(PTY + 分屏 + 启 agent + socket 状态点)只占 15–20%,**其余 80% 全是 shell 集成、resize、粘贴、焦点、恢复这类边角打磨**。kooky 的注释密度极高,等于附赠一份踩坑地图;但这些成本躲不掉,只能靠选路线 A(TUI)把终端渲染那一半外包给用户自己的终端。