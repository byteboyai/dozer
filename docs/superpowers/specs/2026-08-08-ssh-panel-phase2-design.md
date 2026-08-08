# SSH 远程主机面板 · 阶段 2:SSH 终端

**状态:已批准(brainstorming 会话审阅通过,2026-08-08;写计划阶段补了一条
`Handle` 生命周期的验证,见"背景"末尾)**

## 背景

阶段 1(`docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`,已合并
`main`)落地了主机增删改 + 密码/私钥认证 + 异步连接测试 + host key 信任链
(未知主机两步确认、key 变化拒绝)。阶段 1 的设计文档在"背景"里就记下了本
阶段的关键架构发现:

> 本仓库现有的终端渲染管线(`term_model.rs` 的 `TerminalModel`)是**协议无关
> 的纯字节流处理器**……"SSH 终端"阶段大概率可以直接复用现有
> `TerminalModel`/`term_view.rs` 渲染管线,只需把字节来源从"本地 PTY(经
> dozerd)"换成"SSH channel(经 `russh`)"。

阶段 2 就是兑判断:**SSH 终端不进 dozerd、不走新的渲染路径,而是作为一个
新的 tab 后端接入现有终端 tab 体系**。写本文档前已对本仓库现状逐项核实:

- `crates/dozer-app/src/workspace.rs` 的 tab 体系:`SessionTab`(持
  `TerminalModel` + `forwarder: JoinHandle`)+ `pending: HashMap<usize,
  JoinHandle>`(attach 期间任务句柄暂存,`on_tab_attached` 时取出)+
  `forward_events`(把 `TermEvent` 流翻成 `TermOutput`/`SessionExited` 消息)
  + `send_input`/`resize_all`/`close_tab`/`tab_bar`/`terminal_status_bar`,
  这些就是本阶段要复用的全部管线。
- `crates/dozer-app/src/extensions/ssh.rs`(阶段 1 产物):`SshHost`/
  `AuthMethod`/`TestStatus`/`test_connection`/`TestHandler::check_server_key`
  (known_hosts 三态判定)/`keyring_entry`(Keychain 凭据)。
- `russh` 已在依赖里(阶段 1 引入,0.62.5)。阶段 2 用到的 channel API 已
  对照源码核实:`Handle::channel_open_session()`、`Channel::split()` →
  `(ChannelReadHalf, ChannelWriteHalf)`、`ChannelReadHalf::wait()` →
  `Option<ChannelMsg>`(`Data { data: Bytes }`/`ExtendedData`/`Eof`/`Close`)、
  `ChannelWriteHalf::request_pty(want_reply, term, col_width, row_height,
  pix_width, pix_height, terminal_modes)`/`request_shell`/`window_change`/
  `data_bytes`(写半全部 `&self`)。

**写计划阶段补充核实的一条关键约束**:`russh::client::Handle<H>` 必须在
它开出的 `Channel`/`split()` 之后的读写半**存活期间全程不掉**——核对过
`Handle` 自己的 `Drop` 实现(只打一行 debug 日志,没有主动断连逻辑),但
官方 `examples/client_exec_interactive.rs`(随 crate 分发,本机
`~/.cargo/registry` 里能直接读到源码)明确演示的是"`Handle` 作为
`Session` 结构体的字段,和 `channel` 一起活到连接结束"这个写法,不存在
"握手完直接丢弃 `Handle`、只留 `Channel`"这种被验证过的用法。**本阶段
"从 `test_connection` 抽出共享握手段"这一步,抽出来的函数必须返回
`Handle`(而不是提前替调用方开好 channel 再把 `Handle` 丢掉)**,开
channel/split/进泵循环都在拿到 `Handle` 的**同一个函数、同一段生命周期**
里完成,`Handle` 变量全程留在作用域内直到泵循环退出——写计划/实现时按
这条来,不要因为"看着只用得到 `channel`"就提前把 `Handle` scope 收窄。

**阶段 1 已锁定、本阶段直接继承的决策**(不允许回退):

1. SSH 会话生命周期在 `dozer-app` 进程内管理,不经 `dozerd` 托管——关 GUI
   即断开,不跨重启恢复。
2. host key 验证走 `~/.ssh/known_hosts`,与系统 `ssh` 共享信任记录。
3. 未知 host key 不阻塞握手中途等 UI——走"失败回 UI → 信任并重试"两步流程
   (阶段 1 的具体实现选择,同样的模式用于本阶段的"开终端")。
4. host key 变化一律拒绝,没有"跳过验证"选项。

## 目标 / 非目标

**目标**:

1. SSH 主机卡片动作行加"**终端**"按钮:点一下,在当前项目的终端 pane 里
   新开一个 tab,走完 SSH 握手 + 认证 + `channel_open_session` +
   `request_pty` + `request_shell`,落到可交互的远程 shell。
2. **SSH tab 与本地 tab 同构**:进现有 tab 栏(可切换/可关闭/可翻页),
   键盘输入(含 IME、粘贴)、滚动回看、选区复制、窗口 resize 同步、
   `[会话已结束]` 尾行标记,全部走与本地 tab 相同的路径。`SessionTab` 只
   加一个字段 `backend: TabBackend { Daemon, Ssh }` 区分字节流的去向。
3. **读路径零新增消息类型**:连接任务就是泵任务,建立成功后复用现有
   `Message::TabAttached`(合成一份 `SessionInfo`)+ `Message::TermOutput`
   + `Message::SessionExited`——`on_tab_attached`/`TermOutput`/
   `SessionExited` 三个内核处理器的路由语义一字不改。
4. **写路径一条新管道**:`SshOut` mpsc(`Data`/`Resize`),`send_input`/
   查询应答回写/`resize_all` 按 `backend` 分流;关闭 tab = abort 泵任务,
   channel 随任务析构而断开(与本地"abort forwarder + kill 会话"同构)。
5. **开终端也走 host key 确认流程**,且不增加新机制:未知 key 时卡片落
   `UnknownHostKey` 状态(带"信任并重试"),信任后**自动重开这次想开的
   终端**(而不是阶段 1 测试流程的"重新测试连接")——用户意图是开终端,
   "重试"就该重试开终端。key 变化则拒绝,不写 `known_hosts`。
6. 状态栏对 SSH tab 改口:本地 tab 是"dozerd 持有 · 断连可恢复",SSH tab
   是"SSH 直连 · 断连不可恢复"——不把做不到的事说成能做到。

**非目标**(不做,留给后续或永不):

- 不做 SFTP(阶段 3)。
- 不做断线重连、不做会话恢复/跨重启还原(阶段 1 决策 1 的直接推论)。
- 不做端口转发/隧道、不做 `ssh-agent` 集成、不读 `~/.ssh/config`(继承
  阶段 1 非目标)。
- 不做 keepalive 间隔配置、不做连接参数高级项(终端类型固定
  `xterm-256color`)。
- 不做同一主机多 tab 去重——每点一次"终端"开一条独立连接,允许并行多个。

## 架构与数据流

### 1. tab 后端抽象(结构改动全在这两个枚举)

```rust
/// UI → SSH 泵任务的写指令。
pub enum SshOut {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// 一个终端 tab 的字节流去向。
pub enum TabBackend {
    /// dozerd 托管的本地 PTY(现状所有 tab)。
    Daemon,
    /// SSH channel,UI 侧写操作经 `out` 送进泵任务。
    Ssh { out: mpsc::UnboundedSender<SshOut> },
}
```

`SessionTab` 加 `pub backend: TabBackend` 字段,其余字段形状不动。
SSH tab 的 `info: SessionInfo` 是**合成的**(`SessionInfo` 全是 pub 字段,
直接构造,不改它的形状、不加 Option 分支):

- `id`: `format!("ssh:{host_id}")`——与 daemon 会话 id(UUID)永不撞,
  `dispatch_todo_to_existing` 等按 id 匹配的路径天然不会误命中;
- `name`: `format!("ssh: {}", host.name)`——tab 标题(`tab_title` 在
  `agent == Unknown` 且无 OSC 7 cwd 时回落到 `info.name`);
- `agent`: `AgentKind::Unknown`、`agent_state`: `Idle`、`transcript_path`:
  `None`、`cwd`: `"~"`、`project_id`: `Some(project_id)`。

SSH tab 因此天然出现在 Agent 面板的 `Unknown` 分组里、可点选可关闭——
不加特判把它藏起来(它是用户亲手开的会话,藏起来反而难找)。交付检测/
验收/审阅那几条链只由 `AgentStateChanged` 驱动,SSH tab 永远不会收到该
消息,天然不参与,无需屏蔽代码。

### 2. 连接任务 = 泵任务(对齐现有 spawn_new_tab 的单任务结构)

现有本地 tab 的形状:`spawn_new_tab` spawn 一个任务(create → attach →
`forward_events`),任务句柄立刻存进 `pending[tab_id]`,attach 成功后发
`TabAttached`,`on_tab_attached` 从 `pending` 取出句柄装进 `SessionTab.
forwarder`,关 tab 时 `abort()` 这个句柄。SSH 完全同构:

`Workspace::spawn_ssh_tab(io, host_id)`:

1. 从 `self.ssh` 查主机配置;查不到静默放弃。
2. `keyring` 取密码/私钥口令(阶段 1 的 `keyring_entry` 包一个查询函数)。
3. 预分配 `tab_id`(`next_tab_id`),创建 `SshOut` mpsc;`pending[tab_id]`
   存任务句柄、新字段 `ssh_out_pending[tab_id]` 存 sender——两个暂存区
   都在 `on_tab_attached` 里取出(见下)。
4. spawn 任务(10 秒超时罩住"握手 + 认证 + 开 channel"阶段):
   - `connect` + host key 校验(复用阶段 1 `TestHandler`)+ 认证——从
     `test_connection` 里抽出共享的握手段;
   - `channel_open_session()` → `split()` → 写半 `request_pty(true,
     "xterm-256color", cols, rows, 0, 0, &[])` + `request_shell(true)`;
   - **失败**:发 `ssh::Message::TerminalConnectFailed(project_id, host_id,
     tab_id, 文案)`;未知 key/key 变化另外照发阶段 1 已有的
     `UnknownKeyDetected`/`KeyChanged`(内核已有分支路由)。此路**不**发
     `TabAttached`,tab 不产生;
   - **成功**:发 `Message::TabAttached(project_id, tab_id, 合成 SessionInfo,
     空快照)`;随后进入泵循环:

     ```text
     loop {
         select! {
             msg = read_half.wait()  => match msg {
                 Some(Data/ExtendedData) => 发 TermOutput(project_id, tab_id, bytes),
                 Some(Eof | Close)       => 发 SessionExited, return,
                 Some(_)                 => continue,
                 None                    => 发 SessionExited, return,
             },
             out = rx_out.recv() => match out {
                 Some(Data(bytes))          => write_half.data_bytes(bytes),失败即 SessionExited + return,
                 Some(Resize{cols, rows})   => write_half.window_change(cols, rows, 0, 0),
                 None                       => return,   // sender 全被 drop(tab 关了/Workspace 没了)
             }
         }
     }
     ```

   任务结束 = 读写两半析构 = channel 关闭 = SSH 会话断开。关 tab 的
   `forwarder.abort()`、切项目页签时整份 `Workspace` 被丢弃、GUI 退出,
   三条路都落到"任务析构",这就是阶段 1 决策"关 GUI 即断"的物理落点,
   不需要额外清理代码。

### 3. `on_tab_attached` 认领后端

`on_tab_attached` 现逻辑:从 `pending` 取句柄 → `TerminalModel::new` →
push `SessionTab`。只加一步:从 `ssh_out_pending.remove(tab_id)` 取
sender——取得到就是 SSH tab(`backend = Ssh { out }`),取不到就是本地
tab(`backend = Daemon`)。`TabAttached` 消息本身**不加字段、不加新变体**,
本地路径(含启动恢复 `Workspace::bootstrap`)一行不改。

### 4. 内核接线的完整枚举(App::update 里所有要动的分支)

| 分支 | 改动 |
|------|------|
| `Message::Ssh(OpenTerminal(host_id))` | **新增**,拦截在泛型 `Message::Ssh(msg)` 兜底之前:`with_focused_project` → `ws.spawn_ssh_tab(io, host_id)`;同时记下"这台主机信任后要重开终端" |
| `Message::Ssh(TerminalConnectFailed(project_id, host_id, tab_id, err))` | **新增**:清 `pending[tab_id]`/`ssh_out_pending[tab_id]` 两处暂存,再转给 `ssh::update` 落卡片状态(带"不得覆盖 UnknownHostKey/KeyChanged"的同款守卫) |
| `Message::TermOutput` | 查询应答回写从 `client.write` 改成按 `backend` 分流(收敛成一个写入口) |
| `TermInput`/`TermPaste` → `send_input` | 同上,按 `backend` 分流 |
| `PaneResized` → `resize_all` | `Daemon` → `client.resize`;`Ssh` → 发 `SshOut::Resize` |
| `CloseTab` → `close_tab` | `Daemon` 且存活 → `client.kill`(现状);`Ssh` → 什么都不做(abort 句柄即断开) |
| `TabAttached`/`SessionExited`/Agent 列表/tab 栏 | **不改**(SSH tab 经合成 `SessionInfo`/`Unknown` 分组自然兼容) |

写入口收敛:四处写入(`send_input`、`TermOutput` 应答回写、
`dispatch_todo_to_existing`、暂无第五处)统一走一个按 `backend` 分流的
helper——`Daemon` 分支维持 `handle.spawn + client.write` 原样,`Ssh` 分支
`out.send(SshOut::Data(bytes))`(unbounded send 是同步的,不需要 spawn)。

### 5. 视图层分支(就两处)

- `host_card` 动作行加"终端"按钮(挨着"测试连接")。
- `terminal_status_bar`:激活 tab 是 SSH 后端时,尾段文案改
  "SSH 直连 · 断连不可恢复"(状态点/`resume ✓` 那两列照旧——`alive`
  语义对 SSH 同样成立:channel 活着就是活)。

### 6. 信任后重开终端(对阶段 1 两步流程的最小扩展)

`ssh::WorkspaceState` 加一个字段 `reopen_after_trust: Option<String>`
(host id):

- 内核 `OpenTerminal` 分支 spawn 前写入 `Some(host_id)`;
- 连接任务成功与否都**不**清它(分析见下,残留无害);
- `ssh::update` 的 `TrustHostKey` 分支写完 `known_hosts` 后:该字段等于
  本次信任的 host id → 发 `OpenTerminal(id)`(经 `emit` 回到内核拦截分支,
  重走开终端)并清字段;否则维持阶段 1 行为(重发 `TestConnection`)。

残留无害的论证:字段只存一个 host id。假设 X 开终端成功(字段残留 X),
之后 Y 走测试遇到未知 key,用户点信任——`TrustHostKey(Y)` 里字段是 X ≠ Y,
落回 `TestConnection(Y)`,正确;只有"上次开终端遇未知 key、这次信任的就是
它"才触发重开,正是期望行为。

## 错误处理

- 连接层错误分类复用阶段 1 `SshError`(`Russh`/`UnknownHostKey`/
  `KeyChanged`/`AuthFailed`,认证失败已区分"文件不存在"与"需要口令"),
  不新增错误类型;`TerminalConnectFailed` 的文案来自 `SshError` 的
  `Display` + 超时兜底("连接超时(10 秒)")。
- 连接失败落卡片状态行(与测试状态同一列,不新增第二列状态):未知 key →
  `UnknownHostKey` + "信任并重试"按钮;key 变化 → `KeyChanged` 红字;其余 →
  `Err(文案)`。
- 会话中断(对端关 channel/网络断):泵任务发 `SessionExited` →
  `alive = false` → 现有 `[会话已结束]` 标记行,**不为 SSH 单独造文案**
  (少一个分支;文案语义对两者都成立)。
- `out.send` 失败/`data_bytes` 失败:都按"已断开"处理(前者说明泵任务
  已 exit,后者发 `SessionExited` 后退出)。

## 测试策略

单测(无网络):

- `SshOut` 路由:`Ssh` 后端经 helper 写入 → mpsc 对侧收到 `Data`;
  `resize_all` → 收到 `Resize`;
- `close_tab` 对 SSH tab 不触发 daemon kill 路径(构造 `Ssh` tab 断言
  分支走向);
- 合成 `SessionInfo` 的纯函数断言(id 前缀、agent 为 Unknown 等);
- `reopen_after_trust` 判定逻辑(等 → 重开;不等 → 落回测试)。

人工验收(有网络,写入计划 DoD):

- 对 `localhost` sshd(或用户提供的测试机):开终端 → 交互输入/IME/粘贴 →
  窗口缩放远程 `stty size` 跟随 → 关 tab 远端会话消失 → 同一主机开两个
  tab 互不干扰;
- 首次连接新主机:卡片出未知 key 状态 → 信任并重试 → 终端直接开出;
  篡改 `known_hosts` 后重连:拒绝且文案明确;
- 回归:本地 tab 全链路(新建/输入/resize/关闭/启动恢复)行为不变。

## 依赖变更

无。`russh` 0.62.5 已在阶段 1 引入;不新增任何 crate。
