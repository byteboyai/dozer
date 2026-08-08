# SSH 远程主机面板 · 阶段 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `extensions::ssh` 面板:每个项目自己的 SSH 主机列表(增删改)
+ 密码/私钥认证 + 异步连接测试 + 标准 SSH host key 验证(首次连接确认指纹、
之后 key 变了拒绝连接)。这是 SSH 面板 3 个阶段里的第一个,SSH 终端(阶段 2)
和 SFTP 文件管理(阶段 3)都不在这次范围内。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/ssh.rs`。**不需要
App 级 `AppState`**(与数据库面板不同——SSH 没有"多种驱动可开关"这种全局
概念),只有 `WorkspaceState` 挂每个 `Workspace`。连接层用
[`russh`](https://docs.rs/russh/latest/russh/)(纯 Rust,已核实当前版本
API:`client::connect`/`Handle::authenticate_password`/
`authenticate_publickey`/`keys::load_secret_key`/`keys::known_hosts::
{check_known_hosts, learn_known_hosts}`)。Host key 未知时**不**在握手中途
阻塞等 UI 确认(那需要 oneshot channel 跨 async/UI 边界传递,复杂且容易做
错)——改成"测试失败并把服务器公钥带回 UI,用户点‘信任并重试’→ 写入
known_hosts → 重新发起同一次测试"这个更简单、一样能达到设计文档要求效果
的两步流程(这个简化是写计划过程中定的,设计文档本身没规定具体实现机制,
不违反设计意图)。

**Tech Stack:** Rust workspace;`tokio`(已有);新增 `russh`(这次不加
`russh-sftp`——阶段 1 用不上,留到阶段 3 再加,避免引入一个当前完全没用到
的依赖)。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/ssh-panel-phase1`(或对应
  worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于 `main`(截至 commit `b1bfa0c`)分析,当时 `LeftView` 只有
  `Files`/`Web`/`GitLog`/`Todo`/`Project` 5 个变体**。截至写这份计划,还有
  两个并行推进、尚未合并的分支会改到同一批壳层符号:`feature/app-
  workspace-split`(`App`/`Workspace` 拆成 `app.rs`+`workspace.rs`)、
  `feature/database-panel-phase1`(加 `LeftView::Database`)。**开工前用
  `grep -n "pub enum LeftView"` 核对你实际面对的分支上这些符号在哪个文件、
  `LeftView` 已经有几个变体**,不要假设跟本文档写的一致——`Task 5`(接入
  内核)的每一步都是"找到某个既有模式、照着加一份"的形状,不依赖具体行号
  或具体已有多少个变体。
- **这次不做** SSH 终端(交互式 shell)、SFTP 浏览/传输、端口转发、
  `ssh-agent` 集成、从 `~/.ssh/config` 导入——`ssh::view` 只有"主机列表 +
  增删改表单 + 测试连接(含 host key 确认)"这一层。
- **Host key 验证是这次的核心安全逻辑,不能简化掉**:未知主机必须走确认
  流程才能写入 `known_hosts`;已知但不匹配必须拒绝连接并给出明确警告
  文案,不能和"网络不可达"之类的错误混在一起显示成同一种"连接失败"。
- 每个任务结束都要 `cargo build && cargo test && cargo clippy --all-targets
  && cargo fmt` 干净通过(全 workspace)。Task 2/3/4 结束时新模块还没接入
  内核,会有预期中的 `dead_code` 警告,Task 5 解决,不要加 `#[allow(dead_
  code)]`。
- 设计文档:`docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`
  (有疑问以它为准;本计划"Architecture"一节提到的"两步确认流程"是对设计
  文档"首次连接提示信任"这条要求的具体实现选择,不是对设计文档的偏离)。

---

### Task 1: 新增依赖 + 图标资源

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`
- Create: `crates/dozer-app/assets/icons/server.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces:`icons::IconKind::Server` 变体。

- [ ] **Step 1: 加依赖**

`crates/dozer-app/Cargo.toml` 的 `[dependencies]` 块里加:

```toml
russh = { version = "0.51", features = ["russh-cryptovec"] }
```

（版本号按 `cargo add russh` 实际解出的版本为准,不强求锁死 0.51——写这份
计划时 docs.rs 的 `/latest/` 指向的版本用于核实 API,具体 semver 以
`cargo add` 结果为准。`russh-cryptovec` feature 是否需要显式开,视
`cargo add` 默认拉的 features 而定,如果默认就有就不用手动加这行
feature 列表,直接 `russh = "0.51"` 即可。）

- [ ] **Step 2: 装依赖,确认编译**

```bash
cargo build -p dozer-app 2>&1 | tail -30
```

- [ ] **Step 3: 新增图标资源**

创建 `crates/dozer-app/assets/icons/server.svg`(Lucide `server`,去掉
license 注释与 `class` 属性):

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <rect width="20" height="8" x="2" y="2" rx="2" ry="2" />
  <rect width="20" height="8" x="2" y="14" rx="2" ry="2" />
  <line x1="6" x2="6.01" y1="6" y2="6" />
  <line x1="6" x2="6.01" y1="18" y2="18" />
</svg>
```

`IconKind` 枚举加(位置任意,建议挨着其它面板专属图标那一片):

```rust
    /// SSH 主机面板 rail 图标(Lucide server)。
    Server,
```

`bytes()` 的 `match` 里加:

```rust
            IconKind::Server => include_bytes!("../assets/icons/server.svg"),
```

- [ ] **Step 4: 编译/lint/格式确认**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:`IconKind::Server` 这一步还没被构造,有 `dead_code` 警告,Task 5
解决。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock crates/dozer-app/assets/icons/server.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): add russh dep and server icon"
```

---

### Task 2: 数据模型 + 持久化

**Files:**
- Create: `crates/dozer-app/src/extensions/ssh.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod ssh;`)

**Interfaces:**
- Produces:
  - `pub enum AuthMethod { Password, PrivateKey { key_path: String } }`
  - `pub struct SshHost { pub id: String, pub name: String, pub host: String, pub port: u16, pub username: String, pub auth: AuthMethod }`
  - `pub enum TestStatus { Idle, Testing, Ok, UnknownHostKey { fingerprint: String }, KeyChanged { fingerprint: String }, Err(String) }`
  - `pub struct WorkspaceState { .. }`
  - `pub fn hosts_path(repo: &Path) -> PathBuf`
  - `pub fn reload_from_disk(ws_state: &mut WorkspaceState, repo: &Path)`

- [ ] **Step 1: 文件头 + 数据模型**

```rust
// crates/dozer-app/src/extensions/ssh.rs
//! SSH 远程主机面板 · 阶段 1:主机连接管理 + 认证 + 连接测试(含标准 SSH
//! host key 验证)。没有 App 级 `AppState`——SSH 不像数据库面板那样有
//! "驱动类型可开关"的全局概念,只有 `WorkspaceState` 挂每个 `Workspace`。
//! SSH 终端(阶段 2)/SFTP(阶段 3)留后续,见
//! `docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthMethod {
    Password,
    PrivateKey { key_path: String },
}

/// 一个 SSH 主机(不含密码/私钥口令)。`.dozer/ssh_hosts.json` 存
/// `Vec<SshHost>`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshHost {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

/// 某台主机当前的连接测试状态,画在卡片上。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TestStatus {
    #[default]
    Idle,
    Testing,
    Ok,
    /// 首次连接,host key 未知——`fingerprint` 给 UI 显示,用户确认后发
    /// `Message::TrustHostKey(host_id)` 才会真的写入 known_hosts 并重连。
    UnknownHostKey { fingerprint: String },
    /// host key 变了(可能中间人攻击)——直接拒绝,没有"信任并继续"这个
    /// 选项(同设计文档"目标"第 6 条)。
    KeyChanged { fingerprint: String },
    Err(String),
}
```

- [ ] **Step 2: `WorkspaceState`**

```rust
#[derive(Debug, Clone, Default)]
pub struct SshHostDraft {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub use_private_key: bool,
    pub key_path: String,
    pub password: String,
}

/// 挂在每个 `Workspace` 上:当前项目配置的 SSH 主机列表 + 编辑态 + 每台
/// 主机的测试状态 + "未知 host key 待信任"暂存(测试返回
/// `TestStatus::UnknownHostKey` 时,真正的服务器公钥字节存这里,点"信任
/// 并重试"才用得到——不放进 `TestStatus`本身,因为 `TestStatus` 要
/// `PartialEq`/`Clone` 且传给 view 层只需要展示指纹文案,不需要带着一份
/// 公钥字节到处传)。
#[derive(Debug, Default)]
pub struct WorkspaceState {
    hosts: Vec<SshHost>,
    editing: Option<SshHostDraft>,
    test_status: HashMap<String, TestStatus>,
    pending_unknown_keys: HashMap<String, Vec<u8>>,
}

impl WorkspaceState {
    pub fn hosts(&self) -> &[SshHost] {
        &self.hosts
    }
    pub fn editing(&self) -> Option<&SshHostDraft> {
        self.editing.as_ref()
    }
    pub fn test_status(&self, host_id: &str) -> &TestStatus {
        self.test_status.get(host_id).unwrap_or(&TestStatus::Idle)
    }
}

pub fn hosts_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("ssh_hosts.json")
}

pub fn reload_from_disk(ws_state: &mut WorkspaceState, repo: &Path) {
    let path = hosts_path(repo);
    ws_state.hosts = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
}

fn save_hosts(repo: &Path, hosts: &[SshHost]) -> std::io::Result<()> {
    let dir = repo.join(".dozer");
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(hosts).expect("Vec<SshHost> 总能序列化");
    std::fs::write(hosts_path(repo), json)
}

fn keyring_entry(project_id: i64, host_id: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dozer-ssh", &format!("{project_id}:{host_id}"))
}
```

（`keyring::Entry` 服务名用 `"dozer-ssh"` 而不是数据库面板那个
`"dozer"`——两个面板的凭据分开存,互不干扰;这个不是必须的,但更清楚。
`keyring` crate 本身如果数据库面板阶段 1 已经加过依赖,这里不用重复加,
写代码时先 `grep keyring crates/dozer-app/Cargo.toml` 核对现状,已经有就
不用再加一次。）

- [ ] **Step 3: 单测**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str) -> SshHost {
        SshHost {
            id: id.into(),
            name: format!("test-{id}"),
            host: "example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Password,
        }
    }

    #[test]
    fn reload_from_disk_missing_file_yields_empty_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.hosts().is_empty());
    }

    #[test]
    fn save_then_reload_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = vec![host("a"), host("b")];
        save_hosts(dir.path(), &hosts).unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert_eq!(ws_state.hosts(), hosts.as_slice());
    }

    #[test]
    fn private_key_auth_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = host("k");
        h.auth = AuthMethod::PrivateKey {
            key_path: "/home/user/.ssh/id_ed25519".into(),
        };
        save_hosts(dir.path(), &[h.clone()]).unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert_eq!(ws_state.hosts()[0].auth, h.auth);
    }

    #[test]
    fn reload_from_disk_corrupt_file_yields_empty_hosts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(hosts_path(dir.path()), "not json").unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.hosts().is_empty());
    }
}
```

- [ ] **Step 4: 注册模块**

`extensions.rs` 的 `pub mod` 列表按字母序插入 `pub mod ssh;`(具体前后邻居
视当时实际已有哪些模块——写计划时是 `acceptance/browser/files/git_log/
project/todo/usage`,`ssh` 排 `project` 和 `todo` 之间;如果数据库面板的
`pub mod database;` 已经落地,顺序相应调整,字母序本身不变)。

- [ ] **Step 5: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app ssh:: -- --test-threads=1
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add ssh extension data model and persistence"
```

---

### Task 3: 连接测试逻辑(含 host key 验证)+ `Message`/`update`

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Consumes:Task 2 的全部类型。
- Produces:`pub enum Message { .. }`、`pub fn update(ws_state: &mut WorkspaceState, msg: Message, project_id: i64, repo_path: &Path, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`(**没有 `app_state` 参数**——这个模块没有 `AppState`,签名比数据库面板的 `update` 少一个参数)。

- [ ] **Step 1: 自定义错误类型 + `Handler`**

`check_server_key` 需要在"未知主机"和"host key 变了"这两种情况下携带额外
信息(指纹/公钥字节)提前终止握手,所以给 `Handler` 定义一个自己的
`Error` 关联类型:

```rust
#[derive(Debug)]
enum SshError {
    Russh(russh::Error),
    UnknownHostKey { fingerprint: String, key_bytes: Vec<u8> },
    KeyChanged { fingerprint: String },
    AuthFailed,
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshError::Russh(e) => write!(f, "{e}"),
            SshError::UnknownHostKey { .. } => write!(f, "未知主机,需要确认指纹"),
            SshError::KeyChanged { fingerprint } => {
                write!(f, "主机指纹已变化({fingerprint}),可能存在中间人攻击风险,拒绝连接")
            }
            SshError::AuthFailed => write!(f, "认证失败(密码或密钥不正确)"),
        }
    }
}

impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Russh(e)
    }
}

struct TestHandler {
    host: String,
    port: u16,
}

impl russh::client::Handler for TestHandler {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();
        match russh::keys::known_hosts::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => Ok(true),
            Ok(false) => Err(SshError::KeyChanged { fingerprint }),
            Err(_) => {
                // 未知主机(不在 known_hosts 里)。`check_known_hosts` 返回
                // `Ok(bool)`/`Err` 的具体语义(哪种情况是 `Ok(false)`、
                // 哪种是 `Err`)写代码时先跑一个隔离的小实验核实(建一个
                // 空的临时 known_hosts 环境,分别测"全新主机"和"key 对不上
                // 的已知主机"两种输入,看实际落在 `Ok(true)`/`Ok(false)`/
                // `Err` 里的哪一支)——docs.rs 页面没有把这三种状态和三种
                // 返回值的对应关系写清楚,不能凭猜测写死;上面这个 `match`
                // 的分支划分是基于合理推测,写代码时按实验结果调整,不要
                // 不加验证直接照抄。
                Err(SshError::UnknownHostKey {
                    fingerprint,
                    key_bytes: server_public_key
                        .to_bytes()
                        .map_err(|_| SshError::AuthFailed)?,
                })
            }
        }
    }
}
```

- [ ] **Step 2: 连接测试异步函数**

```rust
async fn test_connection(host: SshHost, password: Option<String>) -> Result<(), SshError> {
    let config = std::sync::Arc::new(russh::client::Config::default());
    let handler = TestHandler {
        host: host.host.clone(),
        port: host.port,
    };
    let mut handle = russh::client::connect(config, (host.host.as_str(), host.port), handler).await?;

    let auth_result = match &host.auth {
        AuthMethod::Password => {
            let password = password.unwrap_or_default();
            handle.authenticate_password(&host.username, &password).await?
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
        russh::client::AuthResult::Success => Ok(()),
        russh::client::AuthResult::Failure { .. } => Err(SshError::AuthFailed),
    }
}
```

- [ ] **Step 3: `Message` 枚举**

```rust
#[derive(Debug, Clone)]
pub enum Message {
    AddHostStart,
    EditHostStart(String),
    DraftNameChanged(String),
    DraftHostChanged(String),
    DraftPortChanged(String),
    DraftUsernameChanged(String),
    DraftAuthMethodToggled(bool), // true = 用私钥,false = 用密码
    DraftKeyPathChanged(String),
    DraftPasswordChanged(String),
    DraftSave,
    DraftCancel,
    DeleteHost(String),
    TestConnection(String),
    /// 异步测试结果。带 `project_id`,理由同数据库面板(异步结果不能假设
    /// 聚焦项目没变)。`Result` 的 `Err` 用一个精简过的字符串描述——真正
    /// 携带指纹/公钥字节的是下面 `UnknownKeyDetected`,这两条经常一起触发
    /// (一次测试失败,同时是"未知主机"这个具体原因)。
    TestConnectionResult(i64, String, Result<(), String>),
    /// 测试过程中遇到未知 host key,携带指纹文案(给 UI 显示)+ 公钥原始
    /// 字节(存进 `pending_unknown_keys`,点"信任并重试"时要用)。
    UnknownKeyDetected(i64, String, String, Vec<u8>),
    /// 用户确认信任某台主机的 host key:写入 known_hosts,然后重新发起
    /// 一次 `TestConnection`。
    TrustHostKey(String),
}
```

- [ ] **Step 4: `update` 函数**

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::AddHostStart => ws_state.editing = Some(SshHostDraft::default()),
        Message::EditHostStart(id) => {
            if let Some(h) = ws_state.hosts.iter().find(|h| h.id == id) {
                let (use_private_key, key_path) = match &h.auth {
                    AuthMethod::Password => (false, String::new()),
                    AuthMethod::PrivateKey { key_path } => (true, key_path.clone()),
                };
                ws_state.editing = Some(SshHostDraft {
                    id: Some(h.id.clone()),
                    name: h.name.clone(),
                    host: h.host.clone(),
                    port: h.port.to_string(),
                    username: h.username.clone(),
                    use_private_key,
                    key_path,
                    password: String::new(),
                });
            }
        }
        Message::DraftNameChanged(v) => set_draft(ws_state, |d| d.name = v),
        Message::DraftHostChanged(v) => set_draft(ws_state, |d| d.host = v),
        Message::DraftPortChanged(v) => set_draft(ws_state, |d| d.port = v),
        Message::DraftUsernameChanged(v) => set_draft(ws_state, |d| d.username = v),
        Message::DraftAuthMethodToggled(v) => set_draft(ws_state, |d| d.use_private_key = v),
        Message::DraftKeyPathChanged(v) => set_draft(ws_state, |d| d.key_path = v),
        Message::DraftPasswordChanged(v) => set_draft(ws_state, |d| d.password = v),
        Message::DraftSave => {
            let Some(draft) = ws_state.editing.take() else {
                return;
            };
            if draft.name.trim().is_empty() || draft.host.trim().is_empty() {
                ws_state.editing = Some(draft);
                return;
            }
            let id = draft.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let auth = if draft.use_private_key {
                AuthMethod::PrivateKey {
                    key_path: draft.key_path.trim().to_string(),
                }
            } else {
                AuthMethod::Password
            };
            let host = SshHost {
                id: id.clone(),
                name: draft.name.clone(),
                host: draft.host.trim().to_string(),
                port: draft.port.parse().unwrap_or(22),
                username: draft.username.trim().to_string(),
                auth,
            };
            if let Some(pos) = ws_state.hosts.iter().position(|h| h.id == id) {
                ws_state.hosts[pos] = host;
            } else {
                ws_state.hosts.push(host);
            }
            if !draft.password.is_empty()
                && let Ok(entry) = keyring_entry(project_id, &id)
            {
                let _ = entry.set_password(&draft.password);
            }
            if let Err(e) = save_hosts(repo_path, &ws_state.hosts) {
                tracing::warn!("写入 ssh_hosts.json 失败: {e}");
            }
        }
        Message::DraftCancel => ws_state.editing = None,
        Message::DeleteHost(id) => {
            ws_state.hosts.retain(|h| h.id != id);
            ws_state.test_status.remove(&id);
            ws_state.pending_unknown_keys.remove(&id);
            if let Ok(entry) = keyring_entry(project_id, &id) {
                let _ = entry.delete_credential();
            }
            if let Err(e) = save_hosts(repo_path, &ws_state.hosts) {
                tracing::warn!("写入 ssh_hosts.json 失败: {e}");
            }
        }
        Message::TestConnection(id) => {
            let Some(host) = ws_state.hosts.iter().find(|h| h.id == id).cloned() else {
                return;
            };
            ws_state.test_status.insert(id.clone(), TestStatus::Testing);
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            handle.spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    test_connection(host, password),
                )
                .await;
                match result {
                    Ok(Ok(())) => {
                        emit(Message::TestConnectionResult(project_id, id, Ok(())));
                    }
                    Ok(Err(SshError::UnknownHostKey { fingerprint, key_bytes })) => {
                        // `emit` 是 `Fn`,不是 `FnOnce`——调用它走 `&self`,
                        // 不会把它消费掉,同一个 `async move` 块里连着调
                        // 两次不需要先 `.clone()` 一份。
                        emit(Message::UnknownKeyDetected(
                            project_id,
                            id.clone(),
                            fingerprint.clone(),
                            key_bytes,
                        ));
                        emit(Message::TestConnectionResult(
                            project_id,
                            id,
                            Err(format!("未知主机,指纹 {fingerprint}——需要确认信任")),
                        ));
                    }
                    Ok(Err(e)) => {
                        emit(Message::TestConnectionResult(project_id, id, Err(e.to_string())));
                    }
                    Err(_) => {
                        emit(Message::TestConnectionResult(
                            project_id,
                            id,
                            Err("连接超时(5秒)".to_string()),
                        ));
                    }
                }
            });
        }
        Message::TestConnectionResult(_project_id, id, result) => {
            let status = match result {
                Ok(()) => TestStatus::Ok,
                Err(e) => {
                    // `UnknownKeyDetected` 消息(如果这次测试确实是"未知
                    // 主机"这个原因)已经在下面这个分支之前处理过、把
                    // `UnknownHostKey` 状态写进去了——这里如果状态已经是
                    // `UnknownHostKey` 就不要用泛化的 `Err(String)` 盖掉它。
                    if matches!(ws_state.test_status.get(&id), Some(TestStatus::UnknownHostKey { .. })) {
                        return;
                    }
                    TestStatus::Err(e)
                }
            };
            ws_state.test_status.insert(id, status);
        }
        Message::UnknownKeyDetected(_project_id, id, fingerprint, key_bytes) => {
            ws_state.test_status.insert(id.clone(), TestStatus::UnknownHostKey { fingerprint });
            ws_state.pending_unknown_keys.insert(id, key_bytes);
        }
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
            // 写完 known_hosts,重新发起一次测试——这次 `check_server_key`
            // 应该在 `check_known_hosts` 里能查到刚写入的记录了。
            update(ws_state, Message::TestConnection(id), project_id, repo_path, handle, emit);
        }
    }
}

fn set_draft(ws_state: &mut WorkspaceState, f: impl FnOnce(&mut SshHostDraft)) {
    if let Some(draft) = ws_state.editing.as_mut() {
        f(draft);
    }
}
```

`Message::TestConnection` 处理器里,同一个 `handle.spawn` 的 `async move`
块内,`UnknownHostKey` 那个分支要连着调用 `emit` 两次(先发
`UnknownKeyDetected` 再发 `TestConnectionResult`)——这不需要给 `emit` 加
`Clone` bound:`emit: impl Fn(Message) + ..` 是 `Fn`(不是 `FnOnce`),调用
它走的是 `&self`(`Fn::call(&self, args)`),不会消费掉 `emit` 本身,同一个
`emit` 在同一个闭包里调多少次都行,和数据库面板的 `update` 签名
(`impl Fn(Message) + Send + 'static`)完全一致,不用改。

`PublicKey::from_bytes`/`to_bytes` 这两个方法名写计划时基于"russh 的
`PublicKey` 类型来自 `ssh_key` crate,`ssh_key::PublicKey` 应该有对称的
序列化/反序列化方法"这个合理推测写的,不是从 docs.rs 逐字核实过——写代码
时用 `cargo doc --open -p dozer-app`(或直接看 `~/.cargo/registry/src/
*/ssh-key-*/src/public.rs` 源码)核对这两个方法的准确名字/签名,如果实际
名字不同(比如是 `encode`/`decode` 或者需要走 `ssh_key::Encode`/`Decode`
trait),按实际 API 调整,不要因为编译报错就绕过"存字节、重建 PublicKey"
这个思路本身——这一步的目的(把未知公钥从检测时刻带到"用户点了信任"那一
刻)是设计要求,具体用哪个方法名序列化只是实现细节。

- [ ] **Step 5: 编译/测试/lint/格式**

```bash
cargo build -p dozer-app
cargo test -p dozer-app ssh:: -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:这一步大概率不会一次编译通过——`PublicKey::from_bytes`/`to_bytes`
的具体方法名、`check_known_hosts` 的 `Ok`/`Err` 语义划分,都标注了"按实际
API 核实调整"。按报错信息对照 `russh`/`ssh-key` 实际文档修正,调整后目标
是:未知主机能检测到、host key 不匹配能拒绝、正常匹配能放行——这三个
分支的**行为**不能因为调整方法名而跑偏。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "feat(dozer-app): add ssh connection-test logic with host key verification"
```

---

### Task 4: `view()` 面板视图

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Produces:`pub fn view<'a>(ws_state: &'a WorkspaceState, width: Length, outer: Border) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>`

- [ ] **Step 1: 主机卡片**

```rust
fn host_card<'a>(
    host: &'a SshHost,
    status: &'a TestStatus,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let auth_label = match &host.auth {
        AuthMethod::Password => "密码".to_string(),
        AuthMethod::PrivateKey { key_path } => format!("私钥: {key_path}"),
    };
    let (status_text, status_color) = match status {
        TestStatus::Idle => (String::new(), crate::theme::color::DIM),
        TestStatus::Testing => ("测试中…".to_string(), crate::theme::color::DIM),
        TestStatus::Ok => ("✓ 连接成功".to_string(), crate::theme::color::GREEN),
        TestStatus::UnknownHostKey { fingerprint } => {
            (format!("⚠ 未知主机,指纹 {fingerprint}"), crate::theme::color::GOLD)
        }
        TestStatus::KeyChanged { fingerprint } => {
            (format!("✗ 主机指纹已变化({fingerprint}),拒绝连接"), crate::theme::color::RED)
        }
        TestStatus::Err(e) => (format!("✗ {e}"), crate::theme::color::RED),
    };
    let mut actions = row![
        button(text("测试连接")).on_press(Message::TestConnection(host.id.clone())),
        button(text("编辑")).on_press(Message::EditHostStart(host.id.clone())),
        button(text("删除")).on_press(Message::DeleteHost(host.id.clone())),
    ]
    .spacing(8);
    if matches!(status, TestStatus::UnknownHostKey { .. }) {
        actions = actions.push(
            button(text("信任并重试")).on_press(Message::TrustHostKey(host.id.clone())),
        );
    }
    container(
        column![
            row![
                text(host.name.clone()).size(crate::theme::font::body()).color(crate::theme::color::CREAM),
                text(format!("{}@{}:{}", host.username, host.host, host.port))
                    .size(crate::theme::font::caption_sm())
                    .color(crate::theme::color::DIM),
            ]
            .spacing(8),
            text(auth_label).size(crate::theme::font::caption_sm()).color(crate::theme::color::DIM),
            actions,
            text(status_text).size(crate::theme::font::caption_sm()).color(status_color),
        ]
        .spacing(6),
    )
    .padding(10)
    .width(iced_widget::core::Length::Fill)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(crate::theme::color::CARD.into()),
        border: iced_widget::core::Border {
            color: crate::theme::color::BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}
```

- [ ] **Step 2: 新增/编辑表单**

```rust
fn host_form<'a>(draft: &'a SshHostDraft) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text_input("名字", &draft.name).on_input(Message::DraftNameChanged).size(crate::theme::font::body()),
        text_input("host", &draft.host).on_input(Message::DraftHostChanged).size(crate::theme::font::body()),
        text_input("port(默认 22)", &draft.port).on_input(Message::DraftPortChanged).size(crate::theme::font::body()),
        text_input("username", &draft.username).on_input(Message::DraftUsernameChanged).size(crate::theme::font::body()),
        row![
            button(text(if draft.use_private_key { "● 私钥" } else { "○ 私钥" }))
                .on_press(Message::DraftAuthMethodToggled(true)),
            button(text(if !draft.use_private_key { "● 密码" } else { "○ 密码" }))
                .on_press(Message::DraftAuthMethodToggled(false)),
        ]
        .spacing(8),
    ]
    .spacing(8);

    if draft.use_private_key {
        col = col.push(
            text_input("私钥文件路径,如 ~/.ssh/id_ed25519", &draft.key_path)
                .on_input(Message::DraftKeyPathChanged)
                .size(crate::theme::font::body()),
        );
        col = col.push(
            text_input("私钥口令(留空则不修改/无口令)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(crate::theme::font::body()),
        );
    } else {
        col = col.push(
            text_input("password(留空则不修改)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(crate::theme::font::body()),
        );
    }

    col = col.push(
        row![
            button(text("保存")).on_press(Message::DraftSave),
            button(text("取消")).on_press(Message::DraftCancel),
        ]
        .spacing(8),
    );

    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fill)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(crate::theme::color::CARD.into()),
            border: iced_widget::core::Border {
                color: crate::theme::color::GOLD,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

- [ ] **Step 3: 整合 `view`**

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: iced_widget::core::Length,
    outer: iced_widget::core::Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        row![
            text("SSH 主机").size(crate::theme::font::subtitle()).color(crate::theme::color::CREAM),
            button(text("＋新增主机")).on_press(Message::AddHostStart),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    ]
    .spacing(12);

    if let Some(draft) = ws_state.editing() {
        col = col.push(host_form(draft));
    }

    if ws_state.hosts().is_empty() {
        col = col.push(text("还没有主机").size(crate::theme::font::body()).color(crate::theme::color::DIM));
    } else {
        for h in ws_state.hosts() {
            col = col.push(host_card(h, ws_state.test_status(&h.id)));
        }
    }

    container(col.padding(16))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(crate::theme::color::BG.into()),
            border: outer,
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

（顶部 `use` 块照抄 `extensions::todo.rs`/`extensions::database.rs`(如果
已经落地)的组件导入写法。）

- [ ] **Step 4: 编译/lint/格式**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "feat(dozer-app): add ssh panel view"
```

---

### Task 5: 接入内核

**Files:**
- Modify:`workspace.rs`(若拆分/数据库面板已合并,按符号名分布到对应
  文件——见 Global Constraints)。

**Interfaces:**
- Consumes:Task 2-4 的 `ssh::{WorkspaceState, Message, update, view,
  reload_from_disk}`。

- [ ] **Step 1: `LeftView` 加变体**

`grep -n "pub enum LeftView"` 定位,加一个变体(名字 `Ssh`):

```rust
pub enum LeftView {
    Files,
    Web,
    GitLog,
    Todo,
    Project,
    // 如果数据库面板已经落地,这里还会有 Database,顺序不重要,加进去就行
    Ssh,
}
```

- [ ] **Step 2: 补全穷尽 match**

同数据库面板 Task 5 Step 2 的模式——`grep -n "LeftView::Project =>"`(或
`LeftView::Database =>`,取决于哪些面板先落地)定位所有穷尽
`match .left_view`,每处加一个 `LeftView::Ssh` 分支:纯几何函数(
`preview_content_bounds`/`is_in_preview_column` 这两个函数各自的两处
match)返回 `(0.0, 0.0, 0.0, 0.0)`/`false`(同 `GitLog`/`Todo`/`Project`
——SSH 面板阶段 1 没有"预览列"概念);内容分发大 match(`left_panel_area`)
加:

```rust
   LeftView::Ssh => {
       let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
           return column![].into();
       };
       ssh::view(&ws.ssh, Length::Fill, zone_pane_border(zone, ac)).map(Message::Ssh)
   }
```

- [ ] **Step 3: `RailButton` + rail 图标按钮**

`RailButton` 加 `LeftSsh`。`left_icon_rail` 函数里照抄某个既有面板按钮的
结构加一份:

```rust
        // SSH 主机面板入口。
        MouseArea::new(rail_icon_button(
            icons::IconKind::Server,
            app.left_view == LeftView::Ssh && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftSsh)),
            Message::LeftIconSelect(LeftView::Ssh),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftSsh), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftSsh), false)),
```

- [ ] **Step 4: `Workspace` struct 加字段**

`Workspace` struct 里加:

```rust
    ssh: ssh::WorkspaceState,
```

对应 `Workspace::from_restore` 初始化处加 `ssh: ssh::WorkspaceState::
default(),`。**不需要**改 `App` struct(这个模块没有 `AppState`)。

- [ ] **Step 5: `Message` 加变体**

```rust
    Ssh(ssh::Message),
```

- [ ] **Step 6: `App::update` 分发**

因为没有 `AppState`,这里比数据库面板简单一层——不需要
`loaded_workspace_mut` 那种"同时借两个字段"的写法,可以直接套
`with_focused_project`(同步消息)+ 一个特化的 `with_project(pid, ..)`
分支(`TestConnectionResult`/`UnknownKeyDetected` 这两个带显式
`project_id` 的异步结果,同数据库面板的路由要求——不能假设聚焦项目没变):

```rust
            Message::Ssh(ssh::Message::TestConnectionResult(project_id, host_id, result)) => {
                self.with_project(project_id, move |ws, io| {
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    let repo_path = ws
                        .project
                        .as_ref()
                        .map(|p| std::path::PathBuf::from(&p.path))
                        .unwrap_or_default();
                    ssh::update(
                        &mut ws.ssh,
                        ssh::Message::TestConnectionResult(project_id, host_id, result),
                        project_id,
                        &repo_path,
                        &handle,
                        emit,
                    );
                });
            }
            Message::Ssh(ssh::Message::UnknownKeyDetected(project_id, host_id, fingerprint, key_bytes)) => {
                self.with_project(project_id, move |ws, io| {
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    let repo_path = ws
                        .project
                        .as_ref()
                        .map(|p| std::path::PathBuf::from(&p.path))
                        .unwrap_or_default();
                    ssh::update(
                        &mut ws.ssh,
                        ssh::Message::UnknownKeyDetected(project_id, host_id, fingerprint, key_bytes),
                        project_id,
                        &repo_path,
                        &handle,
                        emit,
                    );
                });
            }
            Message::Ssh(msg) => {
                self.with_focused_project(|ws, io| {
                    let Some(project) = ws.project.as_ref() else {
                        return;
                    };
                    let project_id = project.id;
                    let repo_path = std::path::PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    ssh::update(&mut ws.ssh, msg, project_id, &repo_path, &handle, emit);
                });
            }
```

（**注意排序**:两个特化分支必须排在通配的 `Message::Ssh(msg) => ..`
**之前**,同数据库面板 Task 5 Step 6 强调过的 Rust `match` 顺序要求。
`with_project`/`with_focused_project` 的确切签名——是否已经封装了
`io.handle`/`io.proxy` 这种取法,还是要另外写——写代码时对照 `browser.rs`
里 `Message::Browser(msg) => { self.with_focused_project(|ws, io| { ..
})}` 那一段的真实写法抄,本文档给的是形状示意,不是逐字符保证与现状
一致。）

- [ ] **Step 7: 进面板时重读磁盘**

`Message::LeftIconSelect(v)` 处理器里加:

```rust
                if self.left_view == LeftView::Ssh {
                    self.with_focused_project(|ws, _io| {
                        if let Some(project) = ws.project.as_ref() {
                            ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                        }
                    });
                }
```

- [ ] **Step 8: 编译/测试/lint/格式收敛**

```bash
cargo build -p dozer-app 2>&1 | grep -E "^error"
```

反复跑,按报错补漏(常见:`use crate::extensions::ssh;` 没加进顶部批量
`use crate::extensions::{..};` 列表)。

```bash
cargo build -p dozer-app
cargo test -p dozer-app -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全部干净通过,零警告。

- [ ] **Step 9: Commit**

```bash
git add -u
git commit -m "feat(dozer-app): wire ssh panel into LeftView/kernel"
```

---

### Task 6: 全量验证 + 人工验收

**Files:** 无新改动。

- [ ] **Step 1: 全量构建/测试/lint/格式**

```bash
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

- [ ] **Step 2: 人工验收**

```bash
cargo run -p dozer-app
```

1. 打开项目,点左图标栏新出现的"SSH"图标,面板显示"还没有主机"。
2. 新增一台主机,用密码认证,host 填一个可达且允许密码登录的真实测试
   服务器(或本机如果开了 sshd:`ssh localhost` 能登录的话填
   `127.0.0.1`),保存后点"测试连接"。**如果这是这台主机第一次被任何
   工具连接**(`~/.ssh/known_hosts` 里没有记录),卡片应该显示"未知主机,
   指纹 …"+ "信任并重试"按钮,点击后能看到重新测试并最终显示"✓ 连接
   成功";之后再点"测试连接"应该直接成功,不再提示未知主机。
3. 手动改坏 `~/.ssh/known_hosts` 里刚才那条记录的公钥部分(随便改几个
   字符),再点"测试连接",应该看到"主机指纹已变化…拒绝连接"这条明确
   警告,不是笼统的连接失败;确认没有"信任并继续"这类选项能绕过这条
   拒绝。改完记得把 `known_hosts` 那条记录恢复或删掉,不要把测试用的
   垃圾数据留在系统真实的 `known_hosts` 里。
4. 新增一台私钥认证的主机,私钥路径填一个真实存在的 `~/.ssh/id_ed25519`
   类文件,测试连接能验证私钥认证路径可用(如果私钥有 passphrase,填
   passphrase 那个密码框)。
5. 编辑/删除主机,确认卡片相应更新、`.dozer/ssh_hosts.json` 同步变化。
6. 完全退出重开 app,回到同一项目,确认主机列表还在、密码/私钥口令从
   Keychain 正确取回(测试连接不需要重新输入)。
7. 切到另一个项目,确认主机列表按项目隔离,不是全局共享。

- [ ] **Step 3: 确认分支状态**

```bash
git status
git log --oneline main..HEAD
```

按 Global Constraints 提请代码审阅,审阅通过后再合并——不在这个计划里
自动合并。
