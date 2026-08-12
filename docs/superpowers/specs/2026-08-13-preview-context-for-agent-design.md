# 预览面板上下文对外暴露给 agent 设计（`get_preview_context` MCP tool）

**状态:已批准(brainstorming 会话,2026-08-13)**

## 背景

Dozer 是站在用户(甲方)一侧、agent 中立的验收层,监管的外部 CLI agent 目前有 7 家
(`AgentKind`,`crates/dozer-core/src/protocol.rs:19-29`):Claude、Codebuddy、
Opencode、Codex、Qoder、Kilo、V8agent。这些 agent 目前完全不知道用户在 Dozer 预览
面板(`crates/dozer-app/src/preview.rs`)里正在看哪个文件、看到哪一行——这次要打通
这条通路:agent 能主动查到"用户当前预览的文件路径 + 光标/选中范围"。

`crates/dozer-app/src/preview_state.rs` 现有的持久化只落盘 `active_path`(重启后
认回上次打开的文件),不含光标信息,也不同步给 `dozerd`——这条路径与本次改动无关,
继续保留。

## 目标 / 非目标

**目标**:
1. `dozer-app` 预览面板状态变化(tab 切换/光标移动/选区变化)时同步推给 `dozerd`
   缓存。
2. 新建 `crates/dozer-mcp`,暴露一个只读 MCP tool `get_preview_context`,让外部
   agent 进程按需查询。
3. 覆盖已确认支持 MCP 的四家 agent 的安装器:Claude、Codex、OpenCode、Codebuddy。

**非目标(明确不在这次范围)**:
- 不做"改文件"/"控制面板"等写能力 tool——这次只有一个只读 tool,`dozer-mcp` 后续
  往同一个 crate 加 tool,但不在这份 spec 里设计。
- 不覆盖 Qoder(是否支持 MCP 未确认,brainstorming 会话里明确搁置,等确认后再补)、
  Kilo/V8agent(`dozer-hook` 现有安装器都还没接这两家,主线未打通前不额外先做 MCP
  安装器)。
- 不做 HTTP/SSE 传输——只做 stdio,理由见"关键语义确认"。
- 不做鉴权/信任升级——沿用 UDS socket 本机单用户信任边界,与现有 `CreateSession`
  等请求一致。
- 不持久化预览上下文到磁盘——`dozerd` 侧纯内存缓存,daemon 重启即清空,不与
  `preview_state.rs` 的落盘逻辑合并。

## 关键语义确认(brainstorming 会话定案)

- **目标 agent 是外部 CLI agent,不是内置 agent**:与之前暂停的"内置 agent +
  MCP server + chat 面板"三件套工作([[dozer-builtin-agent-work-paused]])方向
  相邻但服务对象不同——那边是给 Dozer 自己的内置 agent 用,这次是给 Dozer 监管的
  外部 agent 用。会话中确认独立的 agent-core(rig)项目已完工,可以回来接
  MCP server 这一片。
- **选 MCP tool 而非纯 CLI 子命令**:定这个方向前专门核实过"agent 中立"这个身份
  会不会被 MCP-only 打折扣——结论是 Codebuddy 确认支持 MCP,V8agent 是自己基于
  rig 写的、后续可升级支持,Qoder 用得少暂不确认但不构成阻塞,四家已确认覆盖面
  足够,故上 MCP。
- **`dozer-mcp` 定位为独立新 crate,不是 `dozer-hook` 的子命令**:`dozer-hook`
  是"agent hooks 单向上报事件给 dozerd"的定位(`main.rs:120` 原话"单向语义:不读
  应答,发完即走"),跟"向 agent 提供可调用 tool"是相反方向,混在同一个二进制里
  日后 tool 变多会难拆。`dozer-mcp` 是这块此前暂停的 MCP server 工作的第一片,
  之后加 tool 都加在这个 crate 里。
- **stdio 传输而非 HTTP**:与 `dozer-hook` 现有"被 agent 拉起的小二进制"定位一致
  ——agent 在 MCP 配置里登记一条 command,自己拉子进程说 stdin/stdout,不需要
  `dozerd` 新开一个监听端口面对跨进程/鉴权问题。
- **`dozerd` 侧推送(push)而非拉取(pull-through)**:query 时 `dozerd` 反过来问
  `dozer-app` 要实时状态,需要在现有 request/reply 模式上新增"服务端反过来调客户
  端"的能力,改动面明显更大;`dozer-app` 主动推、`dozerd` 缓存,足够满足"人眼定位
  这种低频次变化"的场景,查询端只读缓存,延迟低、改动小。
- **1-indexed 行列**:`iced-code-editor` 内部 0-indexed,对外(MCP tool 输出)转
  1-indexed——匹配 `Read` 工具 `cat -n` 式输出的行业惯例,转换点在 `dozer-app`
  推送前,`dozerd`/`dozer-mcp` 只透传不关心索引底数。
- **无选区时 `start == end == 光标位置`,另加 `has_selection` 布尔位**:不用
  `Option<Range>`表达"有没有选区",统一给一个 range 字段,`has_selection` 单独
  区分语义,agent 侧解析逻辑更简单。
- **无活动文本预览时返回空结果而非错误**:"没开文件"/"当前 tab 是图片、webview
  等无 `CodeEditor` 的非文本预览"对 agent 而言是正常情况,不是异常,返回
  `path: null` + `reason` 字段说明,不用 catch 逻辑。
- **`dozer-mcp` 启动时缺 `DOZER_SESSION_ID` 直接报错退出**:`dozer-mcp` 只应该
  在 dozer 拉起的 PTY 会话里跑,与 `dozer-hook` 现有行为(`main.rs:84`)一致。

## 架构与数据流

```
dozer-app (GUI)                dozerd (daemon)              dozer-mcp (新 crate)
  PreviewPane 状态变化   --push-->  per-project 缓存
  (tab切换/光标移动/选区)          PreviewContext                    ^
                                                                     | UDS 查询
                                                          agent CLI 进程
                                                          (Claude/Codex/OpenCode/
                                                           Codebuddy)
                                                              |
                                                          stdin/stdout MCP JSON-RPC
                                                              |
                                                          dozer-mcp serve（子进程）
                                                          读 DOZER_SESSION_ID 定位 project
```

### 1. `crates/dozer-core/src/protocol.rs` 协议改动

```rust
pub struct PreviewContext {
    pub path: String,           // 绝对路径
    pub start_line: u32,        // 1-indexed
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub has_selection: bool,    // false 时 start==end==光标位置
}

// Request 新增:
UpdatePreviewContext {
    project_id: i64,
    context: Option<PreviewContext>,  // None = 当前无活动文本预览
},
GetPreviewContext {
    project_id: i64,
},

// Reply 新增:
PreviewContext {
    context: Option<PreviewContext>,
},
```

`dozerd` 侧存储:纯内存 `HashMap<project_id, PreviewContext>`,不落盘。同一
`project_id` 只保留最新一次推送(后写覆盖前写)——一个 project 同时只应有一个
活跃 `dozer-app` 窗口在管它的预览面板,不做多窗口合并。

### 2. `crates/dozer-app`:变化时推送

`PreviewPane`(`preview.rs:150`)tab 切换/光标移动/选区变化时组装
`PreviewContext` 推给 `dozerd`:
- 有 `tab.editor` 的活动 tab:用 `editor.selection_range()` 组装
  start/end(见下方 vendor 改动);无选区(`!has_selection()`)时用
  `editor.cursor_position()` 当 start==end。0-indexed 结果 +1 转 1-indexed。
- 无活动 tab,或活动 tab 无 `editor`(图片/webview 类):推 `context: None`。
- 光标/选区变化频率高(方向键连续按),推送前做 trailing-edge 防抖(~250ms 内
  的连续变化只落地最后一次),避免刷屏 UDS——用"nonce + 延迟 spawn 任务"的常见
  防抖手法,不引入新的定时框架/依赖。

### 3. `vendor/iced-code-editor`:新增两个透传 pub 方法

内部逻辑已经在 `Cursor`(`canvas_editor/cursor_set.rs:40,59`)上现成实现,只是
没对外暴露:

```rust
impl CodeEditor {
    pub fn has_selection(&self) -> bool {
        self.cursors.primary().has_selection()
    }
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        self.cursors.primary().selection_range()
    }
}
```

零新逻辑,纯加可见性。已有的 `cursor_position() -> (usize, usize)`
(`canvas_editor/mod.rs:2625`)保持不变,继续用于无选区场景。

### 4. `crates/dozer-mcp`(新 crate,bin: `dozer-mcp`)

- 子命令 `dozer-mcp serve` 启动 stdio MCP server,用现成 Rust MCP SDK(如
  `rmcp`,具体版本到写 plan 时钉死)而非手撸 JSON-RPC——这个 crate 往后要挂更
  多 tool,标准 SDK 省得自己维护协议细节。
- **session → project 解析**:启动时读 `DOZER_SESSION_ID`(缺失即报错退出);
  不在启动时连 `dozerd`(daemon 可能重启,`dozer-mcp` 是长驻进程),等真正收到
  tool call 时才现连 UDS、一次性请求/关闭,不维护长连接。tool call 时先发
  `Request::ListSessions`(复用现有请求,不新增)按 `session_id` 找到对应
  `SessionInfo.project_id`,再发 `GetPreviewContext`。
- **`get_preview_context` tool**:无入参——`project_id` 完全由
  `DOZER_SESSION_ID` 隐式决定,不暴露给 agent 自己填,避免瞎猜。description 面
  向 agent 自己的模型判断"什么时候该调用",内容类似"返回用户当前在 Dozer 预览
  面板里看的文件路径和光标/选中范围;想知道用户正在看哪段代码时调用"。输出
  JSON `{ path, start_line, start_col, end_line, end_col, has_selection,
  reason }`,`path` 为 `null` 时 `reason` 固定为字符串常量
  `"no_active_preview"`——错误处理表格里列的几种"空结果"场景(没开文件、当前
  tab 非文本、`dozerd` 从没收到过推送)v1 统一用这一个值,不做更细的原因区分
  (见"关键语义确认")。

### 5. 安装器(v1 覆盖 Claude/Codex/OpenCode/Codebuddy)

仿 `dozer-hook/src/install.rs` 的幂等 JSON 补丁手法——读各自的 MCP server 配置
文件、追加/更新 `dozer-mcp` 条目、只碰自己写的条目,不影响用户其他配置。Qoder/
Kilo/V8agent 不在 v1 范围(见"非目标")。

## 错误处理

| 情况 | 表现 |
|---|---|
| `dozer-mcp` 启动时缺 `DOZER_SESSION_ID` | 进程直接退出(exit 1) |
| tool call 时 session 在 `dozerd` 里查不到 / `project_id` 为空 | MCP tool 层返回错误,进程不退出,agent 可重试 |
| tool call 时连不上 `dozerd`(daemon 没起/正在重启) | 同上,MCP tool 错误而非进程崩溃 |
| 预览面板没开文件 / 当前 tab 是图片、webview 类无 `CodeEditor` | 空结果 + `reason` |
| `dozerd` 缓存里从没收到过推送(daemon 刚重启,`dozer-app` 还没变化过) | 同上,走同一个空结果路径,不单独区分"从没推过"和"主动关闭" |
| 路径含非 UTF-8 字节 | `to_string_lossy()`,跟 `preview.rs` 里 `flyfish_url`/`encode_component` 现有约定一致,不特殊处理 |

## 测试策略

- **`protocol.rs`**:`PreviewContext`/`UpdatePreviewContext`/`GetPreviewContext`
  的 serde 序列化/反序列化往返测试,跟着现有 `mod tests`(`protocol.rs:261`起)
  的写法。
- **`dozerd`**:push 后 query 拿到刚推的值;未知 `project_id` 返回 `None`;
  同一 `project_id` 连续两次 push 后写覆盖前写。
- **`dozer-app`**:0-indexed→1-indexed 转换单测;防抖逻辑测试(连续快速触发只
  落地最后一次)。
- **vendor `iced-code-editor`**:`has_selection`/`selection_range` 透传方法的
  最小单测(构造已知选区断言返回值)。
- **`dozer-mcp`**:起一个真实/临时 `dozerd` 做集成测试——已知
  `session_id`+`project_id` 时 tool call 返回预期 JSON;缺
  `DOZER_SESSION_ID` 时进程退出码非 0;`session_id` 查不到时返回 MCP 错误而
  不崩溃。
- **安装器**(四家):幂等性测试——跑两次安装只留一条 `dozer-mcp` 条目,仿
  `dozer-hook/src/install.rs` 现有测试手法。
- **人工验收**:真实 Claude Code 或 Codex 会话装上 `dozer-mcp`,预览面板打开
  一个文件并选中几行,让 agent 调用 `get_preview_context`,核对返回的路径和
  行列是否对得上。
- **回归防护**:`cargo build`、`cargo test`(全 workspace,新增 `dozer-mcp`
  纳入)、`cargo clippy --all-targets`、`cargo fmt` 全绿。

## 依赖变更

- 新增 `crates/dozer-mcp`,依赖一个 Rust MCP SDK(候选 `rmcp`,版本到 plan 阶段
  钉死)+ 现有 `dozer-core`(协议类型复用)。
- `vendor/iced-code-editor` 新增两个 pub 方法,无新增外部依赖。
- `dozer-app`/`dozerd` 无新增外部依赖,只是 `protocol.rs` 新增类型 + 现有 UDS
  连接上多两种请求。
