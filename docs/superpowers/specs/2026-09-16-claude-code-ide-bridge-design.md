# Claude Code IDE 桥接:让 Dozer 预览上下文自动喂给 Claude Code

**状态:已批准(brainstorming 会话,2026-09-16)**

## 背景

Claude Code 在 VS Code / JetBrains 里跑时,会自动带上"用户当前在 IDE 里
打开的文件 + 选区"这段上下文,每轮 prompt 都附带,渲染成聊天记录里一行
可展开的 `In <文件名>` 提示条(例如 `basic_report.json5`)。这不是靠某个
可见的 MCP 工具调用出来的,而是 Claude Code CLI 检测到自己跑在受支持的
IDE 终端里之后,每轮自动去问 IDE 侧要这段上下文。

检测机制:IDE 插件在 `~/.claude/ide/<port>.lock`(或
`$CLAUDE_CONFIG_DIR/ide/<port>.lock`)写一份锁文件,内容包含随机端口、
128 位 token、`workspaceFolders`。Claude Code CLI 启动时扫描这个目录,
用 token 去连对应的本地 WebSocket(JSON-RPC 2.0,魔改版 MCP transport,
鉴权 header 是 `x-claude-code-ide-authorization: <token>`),连上后 IDE
侧暴露 `getCurrentSelection`/`getOpenEditors`/`getDiagnostics` 等一批对
模型隐藏的内部工具,CLI 自己调用来拼这行上下文提示,不经过模型决策。

官方文档只公开了这套机制的安全须知,没有正式的协议 schema;第三方项目
`coder/claudecode.nvim` 的 `PROTOCOL.md` 是目前最可信的逆向参考,证明
这套协议可以被第三方复刻(Neovim 已经做出来了)。

Dozer 已经有对应的数据源:`dozerd::preview_context::PreviewContextStore`
按 `project_id` 缓存 `PreviewContext{path, start_line, start_col,
end_line, end_col, has_selection, updated_at_ms}`,由 dozer-app 推、
`dozer-mcp` 的 `get_preview_context` 工具查(手动 MCP 配置这条路已经在
用了)。这次要做的是新增一条"自动上下文"通道,把同一份数据源接到 Claude
Code 的 IDE 握手协议上,而不是重新做一遍数据采集。

## 范围

只做"自动上下文"(对应 `getCurrentSelection`/`getOpenEditors`),**不做**
`getDiagnostics` 的真实实现(占位恒空)、不做 `openDiff`/
`executeCode`/`closeAllDiffTabs` 等编辑类能力——Dozer 的核心裁决是"预览
优先于编辑",这条通道只用来让 Claude Code 知道用户在看哪个文件,不用来
把 Claude Code 的改动通过这条协议推回 Dozer 显示 diff。

默认自动开启,不加设置项开关。协议是半逆向的,未来 Claude Code 升级可能
静默改协议导致这条集成失效——失效的影响面只是"看不到自动上下文提示",
不影响 Claude Code 本身正常使用,可接受,不做侦测/告警。

## 架构

新增 `crates/dozerd/src/ide_bridge.rs`,核心是一个 `IdeBridgeRegistry`,
按 `project_id` 索引。每个 entry 是一个 tokio task:

- 绑定 `127.0.0.1:0`(随机端口)起 WebSocket listener。
- 在锁文件目录(遵循 `$CLAUDE_CONFIG_DIR/ide/`,未设置时回落
  `~/.claude/ide/`)写一份 `<port>.lock`:
  ```json
  {
    "port": 54321,
    "workspaceFolders": ["/path/to/project/root"],
    "ideName": "Dozer",
    "token": "<128-bit hex>"
  }
  ```
  目录权限 `0700`,文件权限 `0600`(仅当前用户可读),token 用系统 CSPRNG
  生成。
- 只读复用 `PreviewContextStore`(不新增写路径),按自己绑定的
  `project_id` 查询。

一个项目对应一份锁文件 + 一个 WebSocket server,跟 VS Code/JetBrains
"一个 IDE 窗口一份锁文件"的模型完全对齐——Claude Code 天然按
`workspaceFolders` 匹配自己的 cwd 去挑锁文件连,不需要 Dozer 自己在一份
锁文件里塞多个项目、猜 CLI 怎么在多项目间路由(这一点在设计阶段评估过
"全局一份锁文件"的方案,因为路由不确定性风险不可控而放弃)。

## 生命周期

挂在 `crates/dozerd/src/session.rs` 会话生死的地方(那里已经知道
`project_id`,也是当前注入 `DOZER_SESSION_ID` 的位置):

- 某项目的活跃会话数 `0 → 1` 时:`IdeBridgeRegistry` 起该项目的 bridge
  (listener + 锁文件)。
- 某项目的活跃会话数 `1 → 0` 时:停止该项目的 bridge(关 listener、删
  锁文件)。
- 不按 `AgentKind` 过滤——会话建立时 agent 类型往往还未知(要等 hook 上报
  才能确定是不是 Claude Code),索性对项目下所有会话一视同仁地起。非
  Claude Code 的 agent 根本不会扫 `~/.claude/ide/`,多起的 WebSocket
  listener 成本可忽略。
- 不做跨 dozerd 重启的持久化,和 `PreviewContextStore` 同一个非目标:
  daemon 重启即清空,下一个会话触发时重新起。
- dozerd 启动时做一次锁文件清扫:遍历锁文件目录,对每份锁文件校验其
  记录的 pid(需要在锁文件里补一个 `pid` 字段,或另用文件名/side-car 记录
  ——具体写法见实现计划)是否还存活,不存活则删除,避免上次崩溃留下的
  假死锁文件误导 Claude Code 去连一个已经不存在的端口。

## 协议桥接

传输层:WebSocket + JSON-RPC 2.0,握手仿 MCP:

1. `initialize` → 返回 `protocolVersion`/`capabilities`。
2. `tools/list` → 只声明三个方法:`getCurrentSelection`、
   `getOpenEditors`、`getDiagnostics`。刻意保留 `getDiagnostics`(哪怕恒
   返回空数组)是因为不确定 Claude Code 侧是否硬依赖这个方法存在于列表
   里才认定"这是个合法 IDE";不声明就直接拒连的风险比多声明一个空实现
   的维护成本更值得规避。
3. `tools/call`:
   - `getCurrentSelection`/`getOpenEditors` → 用连接对应的 `project_id`
     查 `PreviewContextStore.get(project_id)`,查不到(`None`,含 daemon
     刚重启还没收到推送的情况)就返回"无打开文件/无选中"的等价结果,不
     报错——具体字段形状照 Claude Code 协议本身的"无上下文"表示方式
     (待实现阶段对着参考协议确认),处理哲学上对齐
     `dozer-mcp::get_preview_context` 已有的"查不到就返回
     `reason: "no_active_preview"`,不当错误抛"这套思路,而不是字面复用
     它的 JSON 字段名(两边是不同协议,字段名不通用)。
   - `getDiagnostics` → 恒返回空数组。
   - 其余方法(`openDiff`/`executeCode`/`closeAllDiffTabs` 等)不声明、
     不处理,收到就走 JSON-RPC 标准的 method-not-found 错误。
4. **不搬运选中文本内容**——`PreviewContext` 本来就只存路径 + 行列范围,
   不存正文。这条通道只是告诉 Claude Code"用户正看着这个文件这一段",
   真要读取内容,Claude Code 自己有正常的 Read 工具,不需要这条通道额外
   搬一份文件内容过去。

鉴权:入站 WebSocket 连接必须带 `x-claude-code-ide-authorization: <token>`
header,且与该项目锁文件里的 token 一致,不一致直接拒绝握手(关闭连接,
不建立 JSON-RPC 会话)。

## 风险 / 未知项

- `getCurrentSelection`/`getOpenEditors` 的精确 JSON 字段名和结构官方
  没有正式公开文档。实现阶段第一步是对着 `coder/claudecode.nvim` 的
  `PROTOCOL.md` 写出第一版,然后必须拿一个真实的 Claude Code CLI 连上
  Dozer 的 bridge 实测——验证的是"握手成功 + `In <file>` 提示条真的在
  Claude Code 的输出里出现",不是"字段名拼对了就算完成"。
- 协议本身是半逆向的,Claude Code 未来版本可能改传输细节导致这条集成
  静默失效。已确认的取舍:不加开关、不做侦测告警,失效只影响这一个
  体验点。

## 测试策略

不依赖真实 Claude Code 进程、可自动化覆盖的部分:

- 锁文件内容生成(路径解析要认 `$CLAUDE_CONFIG_DIR`,权限位正确)。
- token 生成 + 校验逻辑(合法/非法 header 各自的处理路径)。
- `PreviewContext → JSON-RPC 响应` 的映射函数(含 `None` 时的空结果
  分支)。
- 生命周期逻辑:项目活跃会话数 0↔1 触发 bridge 起停各恰好一次(可以用
  fake registry/假 listener 断言调用次数,不需要真起 WebSocket 连接)。
- 启动时的陈旧锁文件清扫逻辑(给定一批 pid 存活/不存活的锁文件,断言只
  删掉了不存活的那些)。

不能自动化的部分:"Claude Code 真的采纳了这条上下文并显示提示条"这件事
是外部黑盒依赖,归入人工验收清单(比照 Todo/Files/Usage 等既往试点的
惯例),实现计划里要显式写出来,不能靠自动化测试通过就宣称做完。

## 非目标

- 不实现 `getDiagnostics` 的真实诊断能力(恒返回空数组占位)。
- 不实现 `openDiff`/`executeCode`/`closeAllDiffTabs` 等编辑类 IDE 能力
  ——违反 Dozer"预览优先于编辑"的核心裁决,且用户明确只要上下文。
- 不做设置面板开关。
- 不做跨 dozerd 重启的锁文件/连接持久化。
- 不支持"一份锁文件覆盖多个项目"的全局共享模式(评估后因路由不确定性
  放弃,见"架构"一节)。
- 不给非 Claude Code 的 agent(Codex、opencode、Kilo、V8agent 等)做等价
  集成——这套协议是 Claude Code 专属的,其他 agent 不会扫
  `~/.claude/ide/`,天然不受影响,也不需要适配。
