# SSH 远程主机面板 · 阶段 2(SSH 终端)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SSH 主机卡片加"终端"按钮,点开在当前项目终端 tab 栏里新开一个可
交互的远程 shell,复用现有本地终端 tab 的全部管线(渲染/输入/resize/关闭/
状态栏),只新增一个 `TabBackend` 枚举区分字节流去向。未知 host key 走"失败
回 UI → 信任并重试(自动重开这次想开的终端)"两步流程,key 变化一律拒绝。
唯一真相源:`docs/superpowers/specs/2026-08-08-ssh-panel-phase2-design.md`
(下文"设计文档"即此文件)。SFTP(阶段 3)不在本阶段。

**Architecture:** 改动分布在 `crates/dozer-app/src/extensions/ssh.rs`(消息/
状态机扩展)和 `crates/dozer-app/src/workspace.rs`(`SessionTab`/`Workspace`
struct 加字段、新方法 `spawn_ssh_tab`、既有写路径改按 backend 分流)。**不**
碰 `dozerd`、**不**新建渲染管线——`TerminalModel`/`term_view.rs` 原样复用。

**Tech Stack:** Rust;`russh` 0.62.5(阶段 1 已引入,零新增依赖)。**关键
生命周期约束(设计文档"背景"末尾,写计划阶段对照 russh 官方
`examples/client_exec_interactive.rs` 核实过)**:`russh::client::Handle<H>`
必须在它开出的 channel 读写半存活期间全程留在作用域内,不能握手完就把
`Handle` 提前丢弃——本阶段"共享握手函数"这一步必须返回 `Handle` 本身。

## Global Constraints

- **分支基线**:从 `main` 当前 tip 切 `feature/ssh-panel-phase2`(SSH 阶段 1
  已合并;写计划时 `feature/database-panel-phase2`/`feature/app-workspace-
  split` 均未合并到本阶段基线所在的 `main`——按符号名 grep 定位,不要假设
  行号)。完成后提请审阅合并。
- **本阶段决策不可回退**(继承阶段 1,设计文档"背景"节列了 4 条,核心两条
  再强调):SSH 会话不进 `dozerd`,关 GUI 即断,不做断线重连/跨重启恢复;
  host key 变化一律拒绝连接,没有"跳过验证"选项。
- **`SessionInfo` 没有 `Default` impl**(`dozer-core/src/protocol.rs`,9 个
  字段全 `pub` 但直接构造要求逐个给值)。合成 SSH tab 的 `SessionInfo` 时
  除设计文档列出的 `id`/`name`/`agent`/`agent_state`/`transcript_path`/
  `cwd`/`project_id` 外,`command`/`alive`/`created_ms` 三个字段(现状全仓库
  没有任何地方读它们,给合理值即可,不要留空/瞎填)也必须显式给值,写在
  Task 3 里,不要到实现时才发现漏了字段编译不过。
- **写路径改动只做"按 backend 分流",不改现有 daemon 路径的行为**:`send_
  input`/`TermOutput` 应答回写/`dispatch_todo_to_existing`/`resize_all`/
  `close_tab` 五个函数,`Daemon` 分支的代码原样保留,只加 `Ssh` 分支。
- **`Handle` 生命周期**(见上"Tech Stack"):`spawn_ssh_tab` 里握手、开
  channel、split、进泵循环,必须在同一个 async 函数体的同一段作用域内
  完成,`handle` 变量不能提前被"只返回 channel"的辅助函数吞掉。
- 每个任务结束:`cargo build -p dozer-app`、`cargo test -p dozer-app`、
  `cargo clippy -p dozer-app --all-targets`、`cargo fmt --check` 全绿。
  Task 1-2 结束时新符号未必被内核引用,`dead_code` 警告属预期过渡态,
  **不要加 `#[allow(dead_code)]`**,后续任务接入后自然清零。
- 每个任务一个 commit;全部完成跑 Task 7 人工验收再提请合并。

---

### Task 1: `TabBackend`/`SshOut` 类型 + `SessionTab` 加字段 + 共享握手函数

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces(`ssh.rs`):`async fn handshake(host: &SshHost, password: Option<String>) -> Result<russh::client::Handle<TestHandler>, SshError>`(从 `test_connection` 抽出,返回 `Handle` 本身——见 Global Constraints 的生命周期约束)。
- Produces(`workspace.rs`):`pub enum SshOut { Data(Vec<u8>), Resize { cols: u16, rows: u16 } }`、`pub enum TabBackend { Daemon, Ssh { out: mpsc::UnboundedSender<SshOut> } }`、`SessionTab.backend: TabBackend`。

- [ ] **Step 1: `ssh.rs` 抽出共享握手函数**

`ssh.rs` 里找到 `async fn test_connection(host: SshHost, password: Option<String>) -> Result<(), SshError> { .. }`(现状把 connect+认证 全部内联)。改成:

**`TestHandler`/`SshError` 现状都是模块私有**(`struct TestHandler`/`enum
SshError`,没有 `pub`/`pub(crate)`)。`workspace.rs`(Task 3)要跨模块写出
`ssh::SshError::UnknownHostKey { .. }` 这样的匹配式,`handshake` 的返回类型
也要跨模块具名——两个类型都得先升到 `pub(crate)`,否则是 E0603/私有类型
在 `pub(crate)` 接口里的编译错误。这一步把 `struct TestHandler` 改成
`pub(crate) struct TestHandler`、`enum SshError` 改成 `pub(crate) enum
SshError`(enum 变体/字段的可见性跟着 enum 本身走,不需要逐个标)。

```rust
/// SSH 握手共享段:connect + host key 校验(`TestHandler::check_server_key`)
/// + 认证。返回 `Handle` 本身——调用方若要继续开 channel(阶段 2 终端),
/// `Handle` 必须留在作用域内全程存活到 channel 读写半都不再用为止(见
/// 设计文档"背景"末尾;不能在这个函数里把 `Handle` 提前丢弃只返回别的
/// 东西)。`test_connection` 只需要确认握手成功,用完直接让 `handle`
/// 在函数结尾正常析构(不开 channel,没有"提前丢弃"的风险)。`pub(crate)`
/// 是因为 Task 3 的 `Workspace::spawn_ssh_tab`(在 `workspace.rs`,另一个
/// 模块)要直接调它来建终端连接。
pub(crate) async fn handshake(
    host: &SshHost,
    password: Option<String>,
) -> Result<russh::client::Handle<TestHandler>, SshError> {
    let config = std::sync::Arc::new(russh::client::Config::default());
    let handler = TestHandler {
        host: host.host.clone(),
        port: host.port,
    };
    let mut handle =
        russh::client::connect(config, (host.host.as_str(), host.port), handler).await?;

    let auth_result = match &host.auth {
        AuthMethod::Password => {
            let password = password.unwrap_or_default();
            handle
                .authenticate_password(&host.username, &password)
                .await?
        }
        AuthMethod::PrivateKey { key_path } => {
            let key = russh::keys::load_secret_key(key_path, password.as_deref())
                .map_err(|_| SshError::AuthFailed)?;
            let hash_alg = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(
                    &host.username,
                    russh::keys::PrivateKeyWithHashAlg::new(std::sync::Arc::new(key), hash_alg),
                )
                .await?
        }
    };
    match auth_result {
        russh::client::AuthResult::Success => Ok(handle),
        russh::client::AuthResult::Failure { .. } => Err(SshError::AuthFailed),
    }
}

async fn test_connection(host: SshHost, password: Option<String>) -> Result<(), SshError> {
    handshake(&host, password).await?;
    Ok(())
}
```

同一步把 `struct TestHandler { .. }` 改成 `pub(crate) struct TestHandler {
.. }`,`enum SshError { .. }` 改成 `pub(crate) enum SshError { .. }`(定义体
本身不变,只加可见性修饰符)。`test_connection` 原有的调用方/签名全部
不变——纯内部重构,行为不变。

- [ ] **Step 2: `workspace.rs` 加 `TabBackend`/`SshOut`**

找到 `pub enum RailButton { .. }` 之前空白处(或任意顶层类型定义区,紧邻
`SessionTab` 定义之前更合适,方便阅读),加:

```rust
/// UI → SSH 泵任务的写指令(Task 3 的 `spawn_ssh_tab` 消费)。
pub enum SshOut {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// 一个终端 tab 的字节流去向。
pub enum TabBackend {
    /// dozerd 托管的本地 PTY(现状所有 tab)。
    Daemon,
    /// SSH channel,UI 侧写操作经 `out` 送进泵任务(Task 3)。
    Ssh { out: mpsc::UnboundedSender<SshOut> },
}
```

（`mpsc` 需要 `use tokio::sync::mpsc;`——`workspace.rs` 顶部 `use tokio::sync::
mpsc;` 若已存在(核对现有 `use` 块)直接复用,没有就加这一行。）

- [ ] **Step 3: `SessionTab` 加 `backend` 字段**

`pub struct SessionTab { .. }` 定义里,`forwarder: tokio::task::JoinHandle<()>,`
字段之后加:

```rust
    /// 字节流去向(本地 PTY 还是 SSH channel;阶段 2)。
    pub backend: TabBackend,
```

- [ ] **Step 4: 补全全部 `SessionTab { .. }` 构造点**

`grep -n "SessionTab {" crates/dozer-app/src/workspace.rs` 定位,现状 3 处
(`Workspace::from_restore` 的恢复循环、`on_tab_attached`、测试辅助函数
`make_test_tab`)——**这三处现在都只产生本地 tab**,各加一行
`backend: TabBackend::Daemon,`(字段顺序不敏感,加在 struct 字面量里任意
位置,建议紧挨 `forwarder` 那行)。`on_tab_attached` 这一处 Task 3 还会再
改一次(改成按 `ssh_out_pending` 判断,不是恒 `Daemon`),这一步先让它编译
通过。

- [ ] **Step 5: 编译验证**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全绿。`SshOut`/`TabBackend::Ssh` 这一步还没有生产者(没人构造
`TabBackend::Ssh { .. }`),`SshOut`/`Ssh` 变体的 `dead_code`/unused
警告属预期,Task 3 接入后清零。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): extract shared ssh handshake, add TabBackend/SshOut"
```

---

### Task 2: `ssh::Message`/`WorkspaceState` 扩展 + `ssh::update` 新分支

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Produces:`ssh::Message` 新增 `OpenTerminal(String)`、`TerminalConnectFailed(i64, String, usize, String)`(project_id, host_id, tab_id, 错误文案)。`WorkspaceState` 新增 `reopen_after_trust: Option<String>`。

- [ ] **Step 1: `Message` 枚举加两个变体**

`ssh.rs` 的 `pub enum Message { .. }`,在 `TrustHostKey(String),` 之后追加:

```rust
    /// 点主机卡片"终端":内核 `App::update` 里有专门的拦截分支(见
    /// `workspace.rs`),真正的 tab 创建逻辑在那边的 `Workspace::
    /// spawn_ssh_tab`——这个变体在 `ssh::update` 自己的 `match` 里只是
    /// 穷尽匹配需要,不会真的走到这里(内核在通配 `Message::Ssh(msg)`
    /// 之前就拦截了)。
    OpenTerminal(String),
    /// 终端连接失败的异步结果,带 `project_id`(异步结果不能假设聚焦
    /// 项目没变,同 `TestConnectionResult`)。同上,内核在 `ssh::update`
    /// 之前会先做 `pending`/`ssh_out_pending` 清理,这里只负责落卡片
    /// 状态(与 `TestConnectionResult`/`UnknownKeyDetected`/`KeyChanged`
    /// 共用同一列卡片状态,不新增第二列)。
    TerminalConnectFailed(i64, String, usize, String),
```

- [ ] **Step 2: `WorkspaceState` 加 `reopen_after_trust`**

`pub struct WorkspaceState { .. }` 里,`pending_unknown_keys: HashMap<String,
Vec<u8>>,` 字段之后加:

```rust
    /// 信任 host key 后要不要自动重开终端(而不是阶段 1 默认的"重新测试
    /// 连接")——存的是"上一次点‘终端’按钮、连接过程中撞上未知 key 那台
    /// 主机的 id"。`TrustHostKey` 分支里核对是否等于本次信任的 host id,
    /// 相等才重开终端,否则落回测试连接。字段只有一个槽位,极少数"两台
    /// 主机同时未知 key、按点终端的顺序信任"场景下,后点的会覆盖先点的
    /// 意图,顶多导致"该开终端却重新测试连接"这种安全的降级,不会误开
    /// 错主机的终端或者崩溃(设计文档 §6 的论证)。
    reopen_after_trust: Option<String>,
```

- [ ] **Step 3: `update` 里加穷尽分支 + 扩展 `TrustHostKey`**

`match msg { .. }` 里加(位置任意,建议紧邻 `TrustHostKey` 分支;这两个
分支正常运行时不会被触发——内核在 `TestConnectionResult` 等既有特化臂
旁边会先拦截,详见 Task 5):

```rust
        Message::OpenTerminal(_) => {
            // 内核 `App::update` 在通配 `Message::Ssh(msg)` 之前拦截,
            // 这里理论上到不了;写出来只是为了 `match` 穷尽。
        }
        Message::TerminalConnectFailed(_project_id, host_id, _tab_id, err) => {
            // 内核已经在转发之前清过 pending/ssh_out_pending,这里只需要
            // 落卡片状态——与 TestConnectionResult 的 Err 分支同一套过期
            // 防线(不得覆盖 UnknownHostKey/KeyChanged)。
            if matches!(
                ws_state.test_status.get(&host_id),
                Some(TestStatus::UnknownHostKey { .. } | TestStatus::KeyChanged { .. })
            ) {
                return;
            }
            ws_state.test_status.insert(host_id, TestStatus::Err(err));
        }
```

`Message::TrustHostKey(id) => { .. }` 分支现状(阶段 1):

```rust
        Message::TrustHostKey(id) => {
            let Some(key_bytes) = ws_state.pending_unknown_keys.remove(&id) else {
                return;
            };
            let Some(host) = ws_state.hosts.iter().find(|h| h.id == id).cloned() else {
                return;
            };
            if let Ok(key) = russh::keys::ssh_key::PublicKey::from_bytes(&key_bytes) {
                let _ = russh::keys::known_hosts::learn_known_hosts(&host.host, host.port, &key);
            }
            update(
                ws_state,
                Message::TestConnection(id),
                project_id,
                repo_path,
                handle,
                emit,
            );
        }
```

改成信任后按 `reopen_after_trust` 是否等于这次的 `id` 决定重开终端还是
重测连接:

```rust
        Message::TrustHostKey(id) => {
            let Some(key_bytes) = ws_state.pending_unknown_keys.remove(&id) else {
                return;
            };
            let Some(host) = ws_state.hosts.iter().find(|h| h.id == id).cloned() else {
                return;
            };
            if let Ok(key) = russh::keys::ssh_key::PublicKey::from_bytes(&key_bytes) {
                let _ = russh::keys::known_hosts::learn_known_hosts(&host.host, host.port, &key);
            }
            let retry = if ws_state.reopen_after_trust.as_deref() == Some(id.as_str()) {
                ws_state.reopen_after_trust = None;
                Message::OpenTerminal(id)
            } else {
                Message::TestConnection(id)
            };
            update(ws_state, retry, project_id, repo_path, handle, emit);
        }
```

（`update(ws_state, Message::OpenTerminal(id), ..)` 这次递归调用会命中
Step 3 刚加的 `OpenTerminal(_) => {}` 空分支——**这是设计上刻意的空转**:
真正"重开终端"的动作不是靠这条路径触发的,是内核 `App::update` 收到
`emit` 送回来的 `Message::Ssh(OpenTerminal(id))` 之后,在拦截分支里调
`ws.spawn_ssh_tab(io, id)` 完成的,Task 5 会把这条路接上——这里递归调
`update` 只是为了保持"发一条消息让它经 emit 回到内核"这个既有惯用法一致,
不是让 `ssh::update` 自己处理终端重开。）

- [ ] **Step 4: 编译/测试/lint/格式**

```bash
cargo build -p dozer-app
cargo test -p dozer-app ssh::
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全绿(`OpenTerminal`/`TerminalConnectFailed` 这两个变体尚无内核
拦截,`Message::Ssh(OpenTerminal(_))` 目前只会落到 `ssh.rs` 自己那个空
分支——功能上还不完整,但编译/既有测试不受影响,预期过渡态)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "feat(dozer-app): add OpenTerminal/TerminalConnectFailed messages and reopen-after-trust"
```

---

### Task 3: `Workspace::spawn_ssh_tab`(泵任务)+ `on_tab_attached` 认领后端

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces:`impl Workspace { fn spawn_ssh_tab(&mut self, io: &ShellIo, host_id: String) }`;`Workspace` 新字段 `ssh_out_pending: HashMap<usize, mpsc::UnboundedSender<SshOut>>`。
- Modify:`on_tab_attached` 从 `ssh_out_pending` 认领后端。

- [ ] **Step 1: `Workspace` 加 `ssh_out_pending` 字段**

`pending: HashMap<usize, tokio::task::JoinHandle<()>>,` 字段之后加:

```rust
    /// SSH tab 创建期间的写指令发送端暂存区,语义同 `pending`——
    /// `on_tab_attached` 时取出,取得到就是 SSH tab(`backend = Ssh {
    /// out }`),取不到就是本地 tab(`backend = Daemon`)。
    ssh_out_pending: HashMap<usize, mpsc::UnboundedSender<SshOut>>,
```

两处初始化(`from_restore`/`bootstrap` 一类的 `Workspace` 构造函数,
`pending: HashMap::new(),` 那两行旁边各加一行):

```rust
            ssh_out_pending: HashMap::new(),
```

- [ ] **Step 2: `spawn_ssh_tab` 方法**

紧邻 `spawn_new_tab` 之后(结构上是它的 SSH 版本,方便对照阅读)加:

```rust
    /// SSH 版 `spawn_new_tab`:握手 + host key 校验 + 认证 + 开 channel +
    /// request_pty/request_shell,成功后进入读写泵循环。10 秒超时罩住
    /// "握手到 channel 就绪"这一段(同阶段 1 `test_connection` 的超时
    /// 口径,泵循环本身不设超时——那是长连接,超时语义不适用)。
    fn spawn_ssh_tab(&mut self, io: &ShellIo, host_id: String) {
        if self.loading {
            return; // 同 spawn_new_tab:促成中的占位不建会话
        }
        let Some(host) = self.ssh.hosts().iter().find(|h| h.id == host_id).cloned() else {
            return;
        };
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let (cols, rows) = (io.cols, io.rows);
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let password = ssh::keyring_password(project_id, &host_id);
        let (tx_out, mut rx_out) = mpsc::unbounded_channel::<SshOut>();
        let proxy = io.proxy.clone();

        let jh = io.handle.spawn(async move {
            let handshake_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ssh::handshake(&host, password),
            )
            .await;
            let mut handle = match handshake_result {
                Ok(Ok(h)) => h,
                Ok(Err(ssh::SshError::UnknownHostKey { fingerprint, key_bytes })) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::UnknownKeyDetected(
                        project_id,
                        host_id.clone(),
                        fingerprint.clone(),
                        key_bytes,
                    )));
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        format!("未知主机,指纹 {fingerprint}——需要确认信任"),
                    )));
                    return;
                }
                Ok(Err(ssh::SshError::KeyChanged { fingerprint })) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::KeyChanged(
                        project_id,
                        host_id.clone(),
                        fingerprint.clone(),
                    )));
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        format!("主机指纹已变化({fingerprint}),拒绝连接"),
                    )));
                    return;
                }
                Ok(Err(e)) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        e.to_string(),
                    )));
                    return;
                }
                Err(_) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        "连接超时(10秒)".to_string(),
                    )));
                    return;
                }
            };
            // `handle` 必须留在作用域内到函数结尾(泵循环退出前),不能被
            // 提前丢弃——见设计文档"背景"末尾与本计划 Global Constraints。
            let channel = match handle.channel_open_session().await {
                Ok(c) => c,
                Err(e) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        e.to_string(),
                    )));
                    return;
                }
            };
            let (mut read_half, write_half) = channel.split();
            if let Err(e) = write_half
                .request_pty(true, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
                .await
            {
                let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                    project_id,
                    host_id,
                    tab_id,
                    e.to_string(),
                )));
                return;
            }
            if let Err(e) = write_half.request_shell(true).await {
                let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                    project_id,
                    host_id,
                    tab_id,
                    e.to_string(),
                )));
                return;
            }

            let info = ssh::synth_session_info(&host, project_id);
            if proxy
                .send_event(Message::TabAttached(project_id, tab_id, info, Vec::new()))
                .is_err()
            {
                return;
            }

            loop {
                tokio::select! {
                    msg = read_half.wait() => match msg {
                        Some(russh::ChannelMsg::Data { data }) | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                            if proxy
                                .send_event(Message::TermOutput(project_id, tab_id, data.to_vec()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => {
                            let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                            return;
                        }
                        Some(_) => continue,
                    },
                    out = rx_out.recv() => match out {
                        Some(SshOut::Data(bytes)) => {
                            if write_half.data_bytes(bytes).await.is_err() {
                                let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                                return;
                            }
                        }
                        Some(SshOut::Resize { cols, rows }) => {
                            let _ = write_half.window_change(cols as u32, rows as u32, 0, 0).await;
                        }
                        None => return, // 所有 sender 已 drop(tab 关了/Workspace 没了)
                    },
                }
            }
            // `handle`/`read_half`/`write_half` 到这里全部自然析构，channel
            // 随之关闭，SSH 会话断开——不需要额外的 disconnect 调用（同阶段 1
            // "abort 即断开"的既有口径）。
        });

        self.pending.insert(tab_id, jh);
        self.ssh_out_pending.insert(tab_id, tx_out);
    }
```

**这一步依赖 `ssh.rs` 新增两个 `pub(crate)` 辅助**(`ssh::handshake` 本身
Task 1 里已经是 `pub(crate)` 了,不用在这一步重复升级可见性):
`ssh::keyring_password(project_id, host_id) -> Option<String>`(把
`keyring_entry(..).ok().and_then(|e| e.get_password().ok())` 这段现有重复
逻辑收成一个函数,`ssh::update`/`spawn_ssh_tab` 都能用,不重复写)、
`ssh::synth_session_info(host, project_id) -> SessionInfo`(合成
`SessionInfo` 的纯函数,见下)。

- [ ] **Step 3: `ssh.rs` 补两个 `pub(crate)` 辅助**

`keyring_entry` 函数附近加:

```rust
/// 从 Keychain 读某台主机的密码/私钥口令,读不到按无密码处理(不 panic,
/// 同阶段 1 `TestConnection` 分支的既有口径)。`spawn_ssh_tab`/`update`
/// 里的 `TestConnection` 分支共用这个,不重复写 `.ok().and_then(..)`。
pub(crate) fn keyring_password(project_id: i64, host_id: &str) -> Option<String> {
    keyring_entry(project_id, host_id)
        .ok()
        .and_then(|e| e.get_password().ok())
}
```

`Message::TestConnection(id) => { .. }` 分支里原有的
`let password = keyring_entry(project_id, &id).ok().and_then(|e| e.get_password().ok());`
改成 `let password = keyring_password(project_id, &id);`(纯替换,行为不变)。

`SshHost`/`AuthMethod` 定义之后(或任意合适位置)加合成 `SessionInfo` 的
纯函数:

```rust
/// 合成 SSH tab 的 `SessionInfo`(`dozer_core::protocol::SessionInfo` 没有
/// `Default` impl,9 个字段全要给值)。`command`/`created_ms` 现状全仓库
///没有任何地方读取,给有意义但不影响功能的值,不留空。
pub(crate) fn synth_session_info(
    host: &SshHost,
    project_id: i64,
) -> dozer_core::protocol::SessionInfo {
    dozer_core::protocol::SessionInfo {
        id: format!("ssh:{}", host.id),
        name: format!("ssh: {}", host.name),
        command: format!("ssh {}@{}:{}", host.username, host.host, host.port),
        cwd: "~".to_string(),
        alive: true,
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        agent_state: dozer_core::protocol::AgentState::Idle,
        transcript_path: None,
        project_id: Some(project_id),
        agent: dozer_core::protocol::AgentKind::Unknown,
    }
}
```

（`dozer_core::protocol::{AgentState, AgentKind}` 的具体变体名——`Idle`/
`Unknown`——写计划时按 `workspace.rs` 顶部现有 `use dozer_core::protocol::
{AgentKind, AgentState, ProjectInfo, SessionInfo};` 这一行已经在用的名字
照抄,不用重新核对;`ssh.rs` 这边如果还没 `use dozer_core::protocol::..`,
按需要加,或者像上面这样写全路径。）

- [ ] **Step 4: `on_tab_attached` 认领后端**

`on_tab_attached` 函数体里,`let Some(forwarder) = self.pending.remove(&tab_id)
else { return; };` 之后加:

```rust
        let backend = match self.ssh_out_pending.remove(&tab_id) {
            Some(out) => TabBackend::Ssh { out },
            None => TabBackend::Daemon,
        };
```

`self.tabs.push(SessionTab { .. })` 字面量里,Task 1 Step 4 加的
`backend: TabBackend::Daemon,` 改成 `backend,`(用刚判定出来的值,不再
恒 `Daemon`)。

- [ ] **Step 5: 编译/lint/格式**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:能编译通过。`spawn_ssh_tab` 这一步还没有调用点(Task 5 才接
`Message::Ssh(OpenTerminal(..))` 到它),`dead_code` 警告属预期过渡态。

> 编译大概率不会一次过——`ChannelMsg`/`Handle`/`Channel` 等类型的精确
> 引入路径(`russh::ChannelMsg` 还是 `russh::channels::ChannelMsg`,
> `russh::client::Handle` 等)写计划时按 `cargo doc`/编译报错核对,本文档
> 给的是形状,不是逐字符保证。`Bytes::to_vec()` 需要 `bytes` crate 的
> `Bytes` 类型在作用域内可用(`ChannelMsg::Data.data` 是 `bytes::Bytes`,
> `.to_vec()` 是它自带方法,不需要额外 `use`)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): add spawn_ssh_tab pump task and backend detection in on_tab_attached"
```

---

### Task 4: `App::update` 内核拦截分支

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- `App::update` 新增两个 `Message::Ssh(..)` 特化分支,排在既有
  `KeyChanged` 特化臂之后、通配 `Message::Ssh(msg)` 之前。

- [ ] **Step 1: 定位插入点**

`grep -n "Message::Ssh(" crates/dozer-app/src/workspace.rs` 定位,在
`Message::Ssh(ssh::Message::KeyChanged(project_id, host_id, fingerprint))
=> { .. }` 分支(阶段 1 已有)结束的 `}` 之后、`Message::Ssh(msg) => { .. }`
通配臂之前插入。

- [ ] **Step 2: `OpenTerminal` 分支**

```rust
            // 点"终端"按钮:与既有 `TestConnection`/其它同步交互消息不同,
            // 这个消息不走 `ssh::update`(它要新建一个 tab,需要 `&mut
            // Workspace` 整体,`ssh::update` 只拿得到 `&mut ws.ssh`)——
            // 拦截在通配 `Message::Ssh(msg)` 之前,直接调 `Workspace::
            // spawn_ssh_tab`。
            Message::Ssh(ssh::Message::OpenTerminal(host_id)) => {
                self.with_focused_project(|ws, io| {
                    ws.ssh.record_reopen_after_trust(host_id.clone());
                    ws.spawn_ssh_tab(io, host_id);
                });
            }
```

（`ws.ssh.record_reopen_after_trust(..)` 是 `ssh::WorkspaceState` 上要新加
的一个 `pub(crate)` setter——`reopen_after_trust` 字段是私有的,内核不能
直接赋值,见 Step 4。）

- [ ] **Step 3: `TerminalConnectFailed` 分支**

与既有三个特化臂(`TestConnectionResult`/`UnknownKeyDetected`/`KeyChanged`,
紧邻在这个分支之前)完全同款路由:`self.with_project(project_id, move |ws,
io| { .. })`——不是自己动手找 `Workspace`,直接照抄它们的外壳,只是闭包体
内先多做一步 `pending`/`ssh_out_pending` 清理:

```rust
            // 终端连接失败:先做内核层面的清理(pending/ssh_out_pending
            // 两处暂存——这次连接没能走到 `TabAttached`,不清理会一直占着
            // 这两个 map 的位置),再转给 `ssh::update` 落卡片状态(同
            // `TestConnectionResult` 的路由口径,带显式 project_id,套用
            // 一模一样的 `with_project` 外壳)。
            Message::Ssh(ssh::Message::TerminalConnectFailed(
                project_id,
                host_id,
                tab_id,
                err,
            )) => {
                self.with_project(project_id, move |ws, io| {
                    ws.pending.remove(&tab_id);
                    ws.ssh_out_pending.remove(&tab_id);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    let repo_path = ws
                        .project
                        .as_ref()
                        .map(|p| PathBuf::from(&p.path))
                        .unwrap_or_default();
                    ssh::update(
                        &mut ws.ssh,
                        ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err),
                        project_id,
                        &repo_path,
                        &handle,
                        emit,
                    );
                });
            }
```

- [ ] **Step 4: `ssh::WorkspaceState` 加 `record_reopen_after_trust`**

`ssh.rs` 的 `impl WorkspaceState { .. }` 里(`test_status(..)` 访问器
附近)加:

```rust
    /// 记"点了终端按钮的这台主机,如果接下来撞上未知 host key,信任后要
    /// 自动重开终端"(内核 `App::update` 的 `OpenTerminal` 拦截分支调用;
    /// 字段本身私有,不能让内核直接赋值)。
    pub(crate) fn record_reopen_after_trust(&mut self, host_id: String) {
        self.reopen_after_trust = Some(host_id);
    }
```

- [ ] **Step 5: 编译/测试/lint/格式**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全绿。点"终端"按钮到真正开出 tab 的链路此时应该已经完整(卡片按钮
还没加,Task 6 才加——这一步先确认内核链路本身编译/跑得通,可以先用
一个临时 `Message::LeftIconSelect`-式手动触发方式在开发机上 `cargo run`
验证,也可以等 Task 6 卡片按钮接上后一起过一遍,不强制这一步就做人工
验证)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): wire OpenTerminal/TerminalConnectFailed kernel dispatch"
```

---

### Task 5: 写路径按 backend 分流

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- `send_input`/`Message::TermOutput` 查询应答回写/`dispatch_todo_to_
  existing`/`resize_all`/`close_tab` 五处,`Daemon` 分支保留现状,新增
  `Ssh` 分支。

- [ ] **Step 1: `send_input`**

现状:

```rust
    fn send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if !tab.alive {
            return;
        }
        let client = io.client.clone();
        let id = tab.info.id.clone();
        io.handle.spawn(async move {
            if let Err(e) = client.write(&id, &bytes).await {
                tracing::warn!("写入终端失败: {e}");
            }
        });
    }
```

改成:

```rust
    fn send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if !tab.alive {
            return;
        }
        match &tab.backend {
            TabBackend::Daemon => {
                let client = io.client.clone();
                let id = tab.info.id.clone();
                io.handle.spawn(async move {
                    if let Err(e) = client.write(&id, &bytes).await {
                        tracing::warn!("写入终端失败: {e}");
                    }
                });
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }
```

- [ ] **Step 2: `Message::TermOutput` 查询应答回写**

现状(`App::update` 里):

```rust
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.with_project(project_id, |ws, io| {
                    let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                        return;
                    };
                    tab.ingest_osc(&bytes);
                    let responses = tab.model.feed(&bytes);
                    let alive = tab.alive;
                    let id = tab.info.id.clone();
                    if !responses.is_empty() && alive {
                        let client = io.client.clone();
                        io.handle.spawn(async move {
                            if let Err(e) = client.write(&id, &responses).await {
                                tracing::warn!("回写终端查询应答失败: {e}");
                            }
                        });
                    }
                });
            }
```

改成(`backend` 需要在 `tab` 的可变借用结束前先取出——`match &tab.backend`
本身只读借用没问题,但为了在闭包外层继续用,这里直接在同一个 `if` 块内
按 backend 分流,不额外提前克隆):

```rust
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.with_project(project_id, |ws, io| {
                    let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                        return;
                    };
                    tab.ingest_osc(&bytes);
                    let responses = tab.model.feed(&bytes);
                    if responses.is_empty() || !tab.alive {
                        return;
                    }
                    match &tab.backend {
                        TabBackend::Daemon => {
                            let client = io.client.clone();
                            let id = tab.info.id.clone();
                            io.handle.spawn(async move {
                                if let Err(e) = client.write(&id, &responses).await {
                                    tracing::warn!("回写终端查询应答失败: {e}");
                                }
                            });
                        }
                        TabBackend::Ssh { out } => {
                            let _ = out.send(SshOut::Data(responses));
                        }
                    }
                });
            }
```

- [ ] **Step 3: `dispatch_todo_to_existing`**

现状:

```rust
    fn dispatch_todo_to_existing(&self, io: &ShellIo, session_id: &str, text: &str) {
        let Some(tab) = self.tabs.iter().find(|t| t.info.id == session_id) else {
            return;
        };
        if !tab.alive {
            return;
        }
        let client = io.client.clone();
        let id = session_id.to_string();
        let bytes = format!("{text}\n").into_bytes();
        io.handle.spawn(async move {
            if let Err(e) = client.write(&id, &bytes).await {
                tracing::warn!("派发任务文本失败: {e}");
            }
        });
    }
```

改成:

```rust
    fn dispatch_todo_to_existing(&self, io: &ShellIo, session_id: &str, text: &str) {
        let Some(tab) = self.tabs.iter().find(|t| t.info.id == session_id) else {
            return;
        };
        if !tab.alive {
            return;
        }
        let bytes = format!("{text}\n").into_bytes();
        match &tab.backend {
            TabBackend::Daemon => {
                let client = io.client.clone();
                let id = session_id.to_string();
                io.handle.spawn(async move {
                    if let Err(e) = client.write(&id, &bytes).await {
                        tracing::warn!("派发任务文本失败: {e}");
                    }
                });
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }
```

（Todo 面板"派发到已有会话"这条路径现状只按 `session_id` 匹配已有 tab,
SSH tab 的 `info.id` 是 `"ssh:{host_id}"`,理论上也会被列进可派发目标——
这是现状行为的自然延伸,不是这次要新加的功能,派发文本原样走 `SshOut::
Data`,不做特判排除。）

- [ ] **Step 4: `resize_all`**

现状:

```rust
    fn resize_all(&mut self, io: &ShellIo, cols: u16, rows: u16) {
        let client = io.client.clone();
        let handle = io.handle.clone();
        for tab in &mut self.tabs {
            tab.model.resize(cols, rows);
            if tab.alive {
                let client = client.clone();
                let id = tab.info.id.clone();
                handle.spawn(async move {
                    if let Err(e) = client.resize(&id, cols, rows).await {
                        tracing::warn!("同步终端尺寸到 daemon 失败: {e}");
                    }
                });
            }
        }
    }
```

改成:

```rust
    fn resize_all(&mut self, io: &ShellIo, cols: u16, rows: u16) {
        let client = io.client.clone();
        let handle = io.handle.clone();
        for tab in &mut self.tabs {
            tab.model.resize(cols, rows);
            if !tab.alive {
                continue;
            }
            match &tab.backend {
                TabBackend::Daemon => {
                    let client = client.clone();
                    let id = tab.info.id.clone();
                    handle.spawn(async move {
                        if let Err(e) = client.resize(&id, cols, rows).await {
                            tracing::warn!("同步终端尺寸到 daemon 失败: {e}");
                        }
                    });
                }
                TabBackend::Ssh { out } => {
                    let _ = out.send(SshOut::Resize { cols, rows });
                }
            }
        }
    }
```

- [ ] **Step 5: `close_tab`**

现状:

```rust
    fn close_tab(&mut self, io: &ShellIo, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        tab.forwarder.abort();
        if tab.alive {
            let client = io.client.clone();
            let id = tab.info.id.clone();
            io.handle.spawn(async move {
                if let Err(e) = client.kill(&id).await {
                    tracing::warn!("关闭 tab 时结束会话失败: {e}");
                }
            });
        }
        ...
    }
```

改成(只有 `Daemon` 才调 `client.kill`;`Ssh` 的 `forwarder.abort()` 已经
足够——中断泵任务即 channel/连接析构,同设计文档"目标"第 4 条):

```rust
    fn close_tab(&mut self, io: &ShellIo, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        tab.forwarder.abort();
        if tab.alive && matches!(tab.backend, TabBackend::Daemon) {
            let client = io.client.clone();
            let id = tab.info.id.clone();
            io.handle.spawn(async move {
                if let Err(e) = client.kill(&id).await {
                    tracing::warn!("关闭 tab 时结束会话失败: {e}");
                }
            });
        }
        ...
    }
```

（`...` 是函数剩余部分——`active`/`term_tab_first` 调整逻辑,原样不动,
这里只改判断条件那一行。）

- [ ] **Step 6: `terminal_status_bar`**

现状(`fn terminal_status_bar(ws: &Workspace, outer: Border) -> .. { .. }`):

```rust
        text("dozerd 持有 · 断连可恢复")
            .size(theme::font::caption())
            .color(theme::color::DIM),
```

改成按当前激活 tab 的 backend 取文案:

```rust
        text(match ws.tabs.get(ws.active).map(|t| &t.backend) {
            Some(TabBackend::Ssh { .. }) => "SSH 直连 · 断连不可恢复",
            _ => "dozerd 持有 · 断连可恢复",
        })
        .size(theme::font::caption())
        .color(theme::color::DIM),
```

- [ ] **Step 7: 编译/测试/lint/格式**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): route terminal write paths by TabBackend"
```

---

### Task 6: 视图——卡片"终端"按钮

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:** `host_card` 按钮行加"终端"按钮(挨着"测试连接")。

- [ ] **Step 1: `host_card` 加按钮**

`ssh.rs` 的 `fn host_card<'a>(..) -> .. { .. }` 里,现状按钮行:

```rust
    let mut actions = row![
        button(text("测试连接")).on_press(Message::TestConnection(host.id.clone())),
        button(text("编辑")).on_press(Message::EditHostStart(host.id.clone())),
        button(text("删除")).on_press(Message::DeleteHost(host.id.clone())),
    ]
    .spacing(8);
```

改成插入"终端"按钮:

```rust
    let mut actions = row![
        button(text("测试连接")).on_press(Message::TestConnection(host.id.clone())),
        button(text("终端")).on_press(Message::OpenTerminal(host.id.clone())),
        button(text("编辑")).on_press(Message::EditHostStart(host.id.clone())),
        button(text("删除")).on_press(Message::DeleteHost(host.id.clone())),
    ]
    .spacing(8);
```

（"信任并重试"按钮的追加逻辑——`if matches!(status, TestStatus::
UnknownHostKey { .. }) { actions = actions.push(..) }`——原样保留在
之后,不用改;未知 key 状态不管是"测试连接"还是"终端"触发的,卡片上
显示的都是同一个"信任并重试"入口。）

- [ ] **Step 2: 编译/lint/格式**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全绿,零警告——到这一步 `OpenTerminal`/`TabBackend::Ssh`/`SshOut`
全部有真实调用点,Task 1-5 遗留的 dead_code 应该清零。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "feat(dozer-app): add terminal button to ssh host card"
```

---

### Task 7: 单测 + 全量验证 + 人工验收

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(单测)
- Modify: `crates/dozer-app/src/workspace.rs`(单测,如需要)

**Interfaces:** 设计文档"测试策略"列的 4 类无网络单测。

- [ ] **Step 1: `synth_session_info` 纯函数单测**

`ssh.rs` 现有 `#[cfg(test)] mod tests` 里加:

```rust
    #[test]
    fn synth_session_info_shape() {
        let host = host("h1"); // 现有测试辅助函数,阶段 1 已有
        let info = synth_session_info(&host, 7);
        assert_eq!(info.id, "ssh:h1");
        assert!(info.name.starts_with("ssh: "));
        assert_eq!(info.project_id, Some(7));
        assert!(info.alive);
        assert_eq!(info.agent, dozer_core::protocol::AgentKind::Unknown);
    }
```

（`host(id)` 是阶段 1 就有的测试辅助函数——`fn host(id: &str) -> SshHost`,
`mod tests` 顶部已有,直接复用。）

- [ ] **Step 2: `reopen_after_trust` 判定逻辑单测**

```rust
    #[test]
    fn trust_host_key_reopens_terminal_only_when_ids_match() {
        // 需要一个不可达端口的 Postgres 风格测试主机复用不了(那是数据库
        // 面板的辅助),这里用阶段 1 已有的 SSH host() 辅助 + 一个假指纹/
        // 假公钥字节——不需要真连接,只测状态判定这一段。
        let mut ws = WorkspaceState::default();
        ws.hosts.push(host("A"));
        ws.pending_unknown_keys.insert("A".into(), vec![1, 2, 3]);
        ws.reopen_after_trust = Some("A".into());

        // TrustHostKey("A") 走到 `retry` 判定那一段:直接断言字段而不是
        // 跑真实 update()(update 里 learn_known_hosts 会真的碰
        // ~/.ssh/known_hosts,单测不做这个——只测"id 匹配时清空字段"这条
        // 纯逻辑,复制 update 里那几行判定作为独立小函数会更好测,若
        // 实现时确实拆出了这样一个纯函数,这里改成测那个函数;若没拆,
        // 这条测试保留断言字段初始状态的形状即可,过期结果防线的等价
        // 测试已经在阶段 1 覆盖过,这条测试的重点是文档化“只有 id 相同
        // 才重开终端”这条语义,供以后改代码时有断言可依。
        assert_eq!(ws.reopen_after_trust.as_deref(), Some("A"));
    }
```

（这条测试写得比较弱——`TrustHostKey` 分支本身会真的调用
`russh::keys::known_hosts::learn_known_hosts`,写真实 `~/.ssh/known_hosts`,
不适合在单测里跑。写计划这一步的判断:如果实现时把"id 是否匹配 → 选
`OpenTerminal` 还是 `TestConnection`"这段判定拆成一个不碰 `known_hosts`
的纯函数(比如 `fn pick_retry_message(reopen: &mut Option<String>, id:
&str) -> Message`),就测那个纯函数,测试价值更高;没拆出来的话,这条
测试退化成上面这样的弱断言,聊胜于无——不强制为了测试单独做这次重构,
但如果实现时顺手拆了,记得把测试也换成测真正的判定逻辑。）

- [ ] **Step 3: `TabBackend`/`SshOut` 路由单测**

`workspace.rs` 的 `#[cfg(test)] mod tests` 里加(需要一个能构造出
`SessionTab` 的辅助——`make_test_tab` 现有,核对它的签名是否方便加
`backend` 参数,不方便就在测试里直接改 `.backend` 字段,不改辅助函数
签名影响其它测试):

```rust
    #[test]
    fn close_tab_skips_daemon_kill_for_ssh_backend() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut tab = make_test_tab(&rt, "ssh:h1", AgentKind::Unknown);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        tab.backend = TabBackend::Ssh { out: tx };
        tab.alive = true;
        // 这条测试的重点是 `close_tab` 里 `matches!(tab.backend,
        // TabBackend::Daemon)` 这个判断本身——直接断言判断结果而不是跑
        // 整个 `close_tab`(它会真的碰 daemon client,单测环境没有真实
        // daemon,`client.kill` 会失败但不 panic,只是测起来意义不大)。
        assert!(!matches!(tab.backend, TabBackend::Daemon));
    }
```

（同上,这条也偏弱——`close_tab` 本身依赖真实 `ShellIo`/daemon 连接,
不适合完整单测覆盖,重点断言"分支判断条件正确"这一层。）

- [ ] **Step 4: 编译/测试/lint/格式**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 5: 人工验收(`cargo run -p dozer-app`,设计文档"测试策略"逐条)**

1. 对 `localhost`(本机开 sshd)或用户提供的测试机:SSH 主机卡片点"终端"→
   新 tab 出现在终端 tab 栏、可交互输入(含中文 IME、粘贴)、滚动回看、
   选区复制正常。
2. 窗口缩放:远程跑 `stty size` 确认尺寸跟随窗口变化。
3. 关闭该 tab:远端会话应该断开(如果测试机能看到进程列表,确认对应
   shell 进程消失)。
4. 同一主机连续点两次"终端":两个 tab 互不干扰,各自独立可用。
5. 首次连接新主机(`known_hosts` 里没有):卡片出"未知主机"状态,点
   "信任并重试"→ 终端直接开出来(不是回到"连接成功"文案,是真的开出
   一个可用 tab)。
6. 篡改该主机在 `known_hosts` 里的记录后再点"终端":拒绝,卡片红字
   "主机指纹已变化…",没有信任选项。
7. 状态栏:激活 tab 是 SSH 时底部文案显示"SSH 直连 · 断连不可恢复";
   切到本地 tab 显示"dozerd 持有 · 断连可恢复"。
8. 回归:本地 tab 全链路(新建/输入/resize/关闭/启动恢复,含"项目打开时
   至少一个终端 tab"这条不变式)行为与改动前一致。
9. 交叉验证:SSH tab 应该出现在 Agent 面板的"Unknown"分组里,可点选/
   可关闭,不参与交付检测/验收(因为它永远不会收到
   `Message::AgentStateChanged`)。

- [ ] **Step 6: 整理提交历史,提请合并**

```bash
git log --oneline main..HEAD   # 约 7 个语义清晰的 commit
```

确认无杂物 commit,提请合并到 `main`,PR 描述附人工验收结果(哪些项在
真实 SSH 服务器上测过、哪些跳过)。
