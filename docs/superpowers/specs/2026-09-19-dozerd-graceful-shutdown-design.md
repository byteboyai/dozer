# dozerd 优雅停止设计

## 背景与动机

`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md:39` 明确裁决"会话存活:PTY 由常驻 daemon 持有,关窗/崩溃不掉会话",`dozerd` 因此被设计成完全独立于 `dozer-app` GUI 生命周期的常驻后台服务——GUI 关窗、Quit Dozer 菜单都不会、也不应该让 `dozerd` 退出(讨论过程中确认过 macOS 原生 "Quit Dozer" 菜单项直接绑定 AppKit `terminate:` selector,不产生任何 winit 事件回调,本次不改动这条路径,维持现状)。

但现状下,用户唯一能让 `dozerd` 退出的手段是终端 `Ctrl-C` 或系统层面 `kill` 进程——这对普通用户不友好,且在"某个 agent 会话卡死、需要重置守护进程状态"这类故障排查场景下没有 GUI 入口。本设计给顶栏 Settings 弹窗新增一个显式的"停止 dozerd"能力,让用户在需要时主动、体面地停掉守护进程,且不影响"关窗不掉会话"这条已锁定的默认行为——停止是一次用户主动选择的操作,不是被动跟随窗口生命周期。

## 目标 / 非目标

**目标:**

1. Settings 弹窗(`crates/dozer-app/src/extensions/settings.rs`)新增"高级"分区,提供"停止 dozerd"按钮。
2. 点击后二次确认(告知将结束当前所有存活 agent 会话),确认后 `dozerd` 对每个存活会话复用现有的"总结后关闭"流程(与关 tab 走的是同一条路径),全部处理完再真正退出进程。
3. 停止过程中 UI 给出明确的等待态反馈(最长可能接近 1 分钟),停止成功后同一个按钮变为"重新启动 dozerd",点击可重新拉起。
4. `dozerd` 停止期间/之后,其余依赖 UDS 的面板通过一条全局提示条获得统一解释,面板自身复用已有的"连接失败"空态,不逐个新增专门文案。
5. 协议层新增的 `Request::Shutdown` 只处理"停止 daemon 自身",与已有的会话级 `Request::Kill`/`Request::CloseWithSummary` 语义不冲突、不复用错位。

**非目标:**

- 不改动 macOS 原生 "Quit Dozer" 菜单的行为——它依然只退出 GUI 进程,不触碰 `dozerd`。
- 不引入 `dozer-app` 持有 `dozerd` 进程 PID/`Child` 句柄的机制——现状 `spawn_dozerd()`(`crates/dozer-app/src/runtime.rs:15-28`)是 fire-and-forget,`dozerd` 也可能由用户手动启动或未来以系统服务方式常驻,信号方案覆盖不了这些场景,本设计固定走 UDS 协议往返。
- 不做"部分停止"(比如只清空某些会话、保留其它会话继续跑而 daemon 本身不退出)——停止就是全量收尾后整个进程退出,YAGNI。
- 不改 `finalize_session_summary` 现有的总结生成算法、超时时长(60s)、轮询间隔(2s)等既有常量,本设计只是把它并发应用到"当前所有存活会话"这一新场景,不重新设计总结机制本身。
- 不做"停止中途取消"——用户确认后这个操作不可中途撤回,只能等它跑完(或极端情况下自己去终端 kill 进程)。

## 协议层(`crates/dozer-core/src/protocol.rs`)

`Request` 枚举新增一个变体,紧跟 `Kill` 之后:

```rust
/// 请求 daemon 对所有存活会话完成"总结后关闭"收尾,再退出进程自身。
/// 与 `Kill`(杀单个会话)、`CloseWithSummary`(单会话总结后关闭)不同,
/// 这是唯一一个"daemon 进程整体退出"的请求。
Shutdown,
```

`Reply` 不新增变体,复用现有 `Reply::Ok`(成功发起并完成收尾)与 `Reply::Error`(如已经在 draining 中重复收到 `Shutdown` 请求)。

## dozerd 处理逻辑(`crates/dozerd/src/server.rs` + `main.rs`)

### draining 状态位

新增一个随其它 `Arc<...>` 一起 clone 进每个连接处理任务的 `Arc<std::sync::atomic::AtomicBool>`(命名 `draining`),初始 `false`。收到 `Request::Shutdown` 时用 `compare_exchange` 原子地置位;若已经是 `true`(重复请求/并发触发),直接回 `Reply::Error { message: "dozerd 正在停止中" }`,不重复跑收尾流程。

`draining == true` 期间,`Request::CreateSession` 分支在最前面加一道检查,直接回 `Reply::Error { message: "dozerd 正在停止,无法创建新会话" }`;其余只读类请求(`ListSessions`/`ListProjects`/查询类)不受影响,正常处理——draining 窗口内 GUI 其它面板应该表现如常,只有"新建会话"这类写操作被挡。

### 收尾流程

1. 从 `registry` 取当前存活会话 id 快照(`Vec<String>`)。
2. 对每个 id `tokio::spawn` 一份现有的 `finalize_session_summary(...)`(`server.rs:225-290`)——与 `Request::CloseWithSummary` handler 调用的是**同一个函数**,同一组 `CLOSE_WITH_SUMMARY_TIMEOUT`(60s)/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`(2s)常量,不新增专属超时参数。每个任务自带 60s 内部截止时间且各自独立,并发执行时总耗时约等于其中最慢的一个(≈60s 封顶),不需要再套一层外部超时。
3. `futures::future::join_all(handles).await` 等全部任务完成(每个任务已经保证会在自己的 deadline 内 kill 掉对应会话,不会无限挂起)。
4. 收尾完成后,把 `Reply::Ok` **写入并 flush 到当前这条 Shutdown 请求的连接**,确认发送成功后,才触发外层 accept 循环退出信号——顺序不能反,否则可能在客户端读到确认前进程已经退出、连接被提前断开。
5. 退出信号用一个 `Arc<tokio::sync::Notify>`(命名 `shutdown_signal`,同样 clone 进每个连接任务),`serve()` 的 accept 循环从:
   ```rust
   loop {
       let (stream, _) = listener.accept().await?;
       ...
   }
   ```
   改为:
   ```rust
   loop {
       tokio::select! {
           accepted = listener.accept() => { /* 现有分支不变 */ }
           _ = shutdown_signal.notified() => {
               let _ = std::fs::remove_file(socket);
               return Ok(());
           }
       }
   }
   ```
   `main.rs` 的 `tokio::select! { r = serve => r?, _ = ctrl_c => {...} }` 结构不用改,`serve()` 正常返回即可让现有分支接住退出。

### 与 `CreateSession` 并发的时序说明

`draining` 置位和"取会话快照"之间存在一个理论上的窄窗口(置位后、取快照前,极小概率有一个正在处理中的 `CreateSession` 请求还是在 draining 置位前已经通过检查、正在创建新会话)。这个新会话可能没被这次收尾流程覆盖到。这属于已知的、可接受的边缘情况——不为这个概率极低的竞态引入额外的锁或屏障(YAGNI),真出现"停止后又意外多活一个会话"的报告再回来加固。

## dozer-client(`crates/dozer-client/src/lib.rs`)

新增:

```rust
/// 请求 dozerd 对所有存活会话收尾后退出。内部等待时长与 daemon 侧收尾
/// 耗时挂钩(最长约 60s),外层包一个 90s 超时兜底纯通信层面的异常
/// (进程卡死、socket 异常等),超时视为失败,不代表 daemon 一定没停。
/// `roundtrip` 本身已经在收到 `Reply::Error` 时转成 `Err`,这里不需要
/// 再单独匹配一次。
pub async fn shutdown_daemon(&self) -> Result<()> {
    match tokio::time::timeout(Duration::from_secs(90), self.roundtrip(&Request::Shutdown))
        .await
        .map_err(|_| anyhow!("等待 dozerd 停止超时"))??
    {
        Reply::Ok => Ok(()),
        other => bail!("意外应答: {other:?}"),
    }
}
```

（超时后的具体错误文案/类型在实现阶段按仓库现有 `anyhow`/错误处理惯例落地,这里只固定行为契约:90s 是通信层超时,不是业务超时。)

## UI 设计(`crates/dozer-app/src/extensions/settings.rs` + 全局提示条)

### Settings 弹窗新增"高级"分区

紧跟"Git 账户连接"区块下方,新增一个带分隔线的"高级"分区,当前只有一行:

- **默认态**:说明文字("停止后所有正在运行的 agent 会话会结束并生成总结,可随时重新启动") + 按钮"停止 dozerd"。
- **确认态**:点击后弹出复用现有确认弹窗组件,标题"停止 dozerd?",正文动态插入当前存活会话数(打开确认弹窗前先发一次 `ListSessions` 取数):"这会结束当前 N 个正在运行的 agent 会话并生成总结(可能需要约 1 分钟)。"确认/取消两个按钮。若 N 为 0,正文改为"当前没有正在运行的会话,dozerd 会直接停止。"
- **停止中态**:按钮禁用,文案"停止中…(等待会话总结,最长约 1 分钟)",调用 `shutdown_daemon()`。
- **已停止态**:按钮文案变为"重新启动 dozerd",同时(见下)`daemon_error` 提示出现。
- **重启中态**:点击"重新启动 dozerd"后按钮短暂禁用、文案"启动中…",调用现有 `runtime::spawn_dozerd()` 后复用现有 `ensure_daemon()` 那套探活轮询确认真正起来了,成功后回到默认态、提示条消失;探活多次失败给出内联报错但保留"重新启动"按钮可再次点击。
- **失败态**(`shutdown_daemon()` 返回 Err,通常是 90s 通信超时):按钮恢复默认态可点击,内联展示"停止请求未确认完成,dozerd 可能仍在运行或已经停止,可以重试或手动检查",不擅自假设成功或失败。

### 复用 `App.daemon_error`

仓库已经有一个专门表达"daemon 连不上"的 App 级共享状态:`App.daemon_error: Option<String>`(`crates/dozer-app/src/app/app.rs:266`),其文档注释明确写着这是"整个程序共享的状态"。它目前有两处渲染:没打开任何项目时的空态提示(`app/view.rs:72`)与有工作区时终端面板顶部的警示条(`term/terminal.rs:63`);"打开项目失败""删除项目失败"等既有场景都是直接 `self.daemon_error = Some(...)` 复用这套展示,不新建组件。本设计同样复用它,不新造 footbar 段落或独立提示条组件:

- 停止成功后:`self.daemon_error = Some("dozerd 已停止,部分功能不可用".into())`。
- 重新启动并探活成功后:`self.daemon_error = None`。

**不做的事**:不引入一套主动心跳/探活机制去检测"用户在终端手动 kill 了 dozerd"这类本设计之外发生的断连——现状 `daemon_error` 只在几个具体失败路径(开项目失败、删项目失败、启动探活失败)里被动设置,本设计只新增"停止/重新启动"这两个动作各自对这个字段的读写,不扩大它的置位来源范围(YAGNI)。

### 其它面板

Agent 会话、文件树、Git Log、用量统计等面板不做代码改动——它们各自已有的"UDS 请求失败"错误处理路径在 `dozerd` 停止后自然会被触发,本设计只保证全局提示条给出统一解释,不逐个面板新写空态文案。

## 错误处理

| 场景 | 处理 |
|---|---|
| 重复点击"停止"(已在 draining 中) | dozerd 侧 `Reply::Error`,GUI 内联展示"已经在停止中",按钮维持"停止中"态不重置 |
| `shutdown_daemon()` 90s 通信超时 | 见上文"失败态",不假设成功/失败,提示用户重试或手动检查 |
| 收尾期间某会话的总结生成失败(`finalize_session_summary` 内部已有的 heuristic 兜底也失败,如落库出错) | 复用现有实现里的 `tracing::error!` 记录,不阻塞整体收尾流程(现有 `finalize_session_summary` 本就是"尽力而为",记日志后继续 kill),不额外反映到 GUI 层面 |
| draining 期间用户尝试新建会话/tab | `CreateSession` 收到 `Reply::Error`,GUI 走现有的"创建会话失败"报错路径,文案里包含"dozerd 正在停止" |
| "重新启动"探活多次失败 | 内联报错,保留按钮可重试,不自动无限重试 |

## 测试策略

- `dozerd` 侧单测(`crates/dozerd`,风格同 `registry.rs` 现有测试):
  - 0 会话时 `Request::Shutdown` 直接收尾退出。
  - 多个会话并发收尾,验证每个会话都触发了 `finalize_session_summary` 且最终被 kill。
  - draining 期间 `Request::CreateSession` 返回 `Reply::Error`。
  - 重复 `Request::Shutdown` 返回 `Reply::Error`。
  - 复用 `finalize_session_summary` 现有测试基础设施对 `timeout`/`poll_interval` 的短间隔注入方式,验证"总结超时走 heuristic 兜底"这条路径在并发场景下依然成立。
- `dozer-client` 侧单测:`shutdown_daemon()` 成功路径;通信超时路径(可用一个假实现的慢速/无响应 UDS server 触发)。
- GUI 侧单测:Settings"高级"分区按钮的状态机(默认→确认中→停止中→已停止→重启中→默认,以及失败态的回退),风格同其它 extension 模块现有的 update/state 测试。
- 人工验收清单:
  1. 开 2-3 个不同 agent 的会话,点"停止 dozerd",确认弹窗文案里的会话数正确,确认后等待期间其它面板仍可正常浏览(只是新建会话被挡),完成后检查每个会话确实生成了总结记录。
  2. 停止完成后确认 `daemon_error` 提示出现(终端面板顶部警示条/空态提示,视当前是否有打开的项目而定),依赖 dozerd 的面板显示预期的"连接失败"空态。
  3. 点"重新启动 dozerd",确认真正拉起且 `daemon_error` 提示消失。
  4. Quit Dozer 菜单退出 GUI 前后,用 `ps`/`Activity Monitor` 确认 `dozerd` 进程原样存活,未被本次改动意外影响。
