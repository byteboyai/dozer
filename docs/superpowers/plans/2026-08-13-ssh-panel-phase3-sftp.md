# SSH 主机面板 阶段 3:SFTP 文件传输 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 点主机卡片"文件传输"图标,在 SSH 面板自己的 tab 条里开一个
SFTP tab,内容是左右两栏文件树(本地项目 / 远程主机),选中文件右键
上传/下载。

**Architecture:** 依赖
`docs/superpowers/plans/2026-08-13-ssh-panel-phase4.md` 已经落地的
`SshTabKind`(`Sftp` 变体)、`OpenSshTab`/`CloseSshTab`/`SelectSshTab`
消息、`ssh_tab_bar`/`ssh_terminal_pane` 渲染骨架。新增
`extensions/ssh/sftp.rs` 子模块装 `RemoteTree`/`SftpTabState`/
`SftpMessage`,新增 `Workspace.sftp_tabs: HashMap<String, SftpTabState>`
(独立于阶段 4 的 `ssh_tabs`——SFTP tab 没有 `TerminalModel`,不是字节流
查看器)。本地文件树复用 `crate::project::FileTree`,远程文件树用
`russh-sftp`(已核实版本 2.4.0 的真实 API:`SftpSession::new(stream)`
接一个来自 `channel.into_stream()` 的流、`read_dir(path)` 返回同步
`Iterator<Item = DirEntry>`、整文件传输直接用 `session.read(path)` /
`session.write(path, &bytes)`,不需要手动管理 `File` 句柄)。

**Tech Stack:** Rust、`russh-sftp 2.4`(新增依赖,与既有 `russh 0.62.5`
搭配使用)。

**Spec:** `docs/superpowers/specs/2026-08-13-ssh-panel-phase3-sftp-design.md`

## Global Constraints

- **前置依赖**:必须在阶段 4(`docs/superpowers/plans/2026-08-13-ssh-panel-phase4.md`)
  合并进 `main` 之后再开工——本计划直接使用阶段 4 新增的
  `SshTabKind`/`ws.ssh_tabs`/`ssh_tab_bar`/`ssh_terminal_pane`/
  `OpenSshTab` 内核拦截分支(阶段 4 里 `OpenSshTab(_, SshTabKind::Sftp)`
  分支是空的 `{}`,本计划把它接上真实逻辑)。
- **分支要求**:独立 worktree/分支(建议 `feature/ssh-panel-phase3-sftp`),
  不直接在 `main` 上改;全部任务完成、测试通过后走代码审阅,通过再
  合并。
- ByteBoy2077 配色沿用 `theme::color` 现有 token,不新增色值。
- 每个 SFTP tab 独立新建一条 SSH 连接(不复用同一主机终端 tab 的连接)
  ——已在 brainstorming 阶段确认,不要"优化"成连接复用。
- 远程文件树操作范围:只浏览(展开目录)+ 上传/下载。不做远程
  mkdir/删除/重命名的独立入口(上传过程中按需 `create_dir` 创建目标
  路径不算——那是"上传"这一个操作的实现细节,不是独立的文件管理入口)。
- `russh_sftp::client::SftpSession` 的公开 API(已用 docs.rs 核实,
  crate 版本 2.4.0):
  - `SftpSession::new(stream) -> SftpResult<Self>`,`stream` 要求
    `AsyncRead + AsyncWrite + Unpin + Send + 'static`。
  - `session.read_dir(path: impl Into<String>) -> SftpResult<ReadDir>`
    ——`ReadDir` 是同步 `Iterator<Item = DirEntry>`(不是 `Stream`,不需要
    `.next().await`,直接 `.collect()`或 `for entry in read_dir`)。
  - `DirEntry::file_name(&self) -> String`、
    `DirEntry::file_type(&self) -> FileType`(`FileType::is_dir(&self)
    -> bool`)。
  - 整文件读写走最简 API,不手动开 `File` 句柄:
    `session.read(path: impl Into<String>) -> SftpResult<Vec<u8>>`、
    `session.write(path: impl Into<String>, data: &[u8]) ->
    SftpResult<()>`。
  - `session.create_dir(path: impl Into<String>) -> SftpResult<()>`。
  - 从 russh `Channel` 拿到 `SftpSession::new` 要的 stream:
    `channel.request_subsystem(true, "sftp").await?` 之后
    `channel.into_stream()`(`ChannelStream<S>`,满足
    `AsyncRead+AsyncWrite+Unpin+Send+'static`)。

---

### Task 1: 新增 `russh-sftp` 依赖

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`

- [ ] **Step 1: 加依赖**

`crates/dozer-app/Cargo.toml` 里 `russh = "0.62.5"` 那一行之后插入:

```toml
# SFTP 客户端(阶段 3):在 russh channel 上跑 SFTP 子系统,查过 docs.rs
# 确认 2.4.0 的公开 API(SftpSession::new(stream)/read_dir/read/write/
# create_dir)与本项目 russh 0.62 线可以直接搭配(russh_sftp::client::
# SftpSession::new 只要求 stream 满足 AsyncRead+AsyncWrite+Unpin+Send+
# 'static,不绑定具体 russh 版本)。
russh-sftp = "2.4"
```

- [ ] **Step 2: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 编译成功,`Cargo.lock` 新增 `russh-sftp` 及其传递依赖的解析
条目。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
build(dozer-app): add russh-sftp dependency for SFTP file transfer

EOF
)"
```

---

### Task 2: `extensions/ssh/sftp.rs` —— `RemoteEntry`/`RemoteTree` 数据模型 + 单测

**Files:**
- Create: `crates/dozer-app/src/extensions/ssh/sftp.rs`
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(顶部加
  `pub mod sftp;`)

**Interfaces:**
- Produces: `sftp::RemoteEntry { path: String, name: String, is_dir:
  bool }`、`sftp::RemoteTree`(`new`/`toggle`/`set_children`/
  `visible_rows`)。

- [ ] **Step 1: 写数据模型(先写没有真实 SFTP IO 的纯逻辑部分)**

```rust
//! SFTP 文件传输(阶段 3):数据模型 + 驱动逻辑,独立子模块避免
//! `extensions/ssh.rs` 继续膨胀(镜像 `extensions/project/links.rs`
//! 的既有拆分先例)。

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// 远程文件树状态:展开集合 + 子项缓存,镜像 `crate::project::FileTree`
/// 的形状,但独立成自己的类型——本地是同步 `std::fs::read_dir`,远程是
/// 异步 SFTP `readdir` 结果回填,两者填充时机不同,不共用同一个类型。
#[derive(Debug, Default)]
pub struct RemoteTree {
    root: String,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<RemoteEntry>>,
    /// 展开但读取失败的目录 → 错误文案,渲染层据此显示"⚠ 无法读取"而
    /// 不是无限 loading。
    errors: HashMap<String, String>,
}

impl RemoteTree {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            ..Self::default()
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// 展开/收起某个远程目录。已展开 → 收起(返回 `None`,不需要重新
    /// 请求)。未展开且缓存里没有 → 展开并返回 `Some(path)`(调用方据此
    /// 发起一次异步 `readdir`)。未展开但已有缓存(比如收起后再展开)
    /// → 展开但返回 `None`(不重复请求,直接用缓存)。
    pub fn toggle(&mut self, dir: &str) -> Option<String> {
        if self.expanded.remove(dir) {
            return None;
        }
        self.expanded.insert(dir.to_string());
        if self.children.contains_key(dir) {
            None
        } else {
            Some(dir.to_string())
        }
    }

    /// 异步 `readdir` 结果回填。`Err` 时记进 `errors`,不清 `expanded`
    /// (目录仍显示为"已展开",只是子项区域显示错误文案)。
    pub fn set_children(&mut self, dir: &str, result: Result<Vec<RemoteEntry>, String>) {
        self.errors.remove(dir);
        match result {
            Ok(mut entries) => {
                entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                });
                self.children.insert(dir.to_string(), entries);
            }
            Err(e) => {
                self.errors.insert(dir.to_string(), e);
            }
        }
    }

    /// 渲染用可见行,形状对齐 `crate::project::TreeRow`,复用同一个行
    /// 渲染函数(见 Task 5)。深度优先遍历 `expanded` 集合,只展开
    /// `expanded` 里存在的目录。
    pub fn visible_rows(&self) -> Vec<crate::project::TreeRow> {
        let mut rows = Vec::new();
        self.push_children(&self.root, 0, &mut rows);
        rows
    }

    fn push_children(&self, dir: &str, depth: usize, out: &mut Vec<crate::project::TreeRow>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        for entry in entries {
            let expanded = self.expanded.contains(&entry.path);
            out.push(crate::project::TreeRow {
                path: std::path::PathBuf::from(&entry.path),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
            });
            if entry.is_dir && expanded {
                self.push_children(&entry.path, depth + 1, out);
            }
        }
    }

    /// 某个已展开目录读取失败时的错误文案。
    pub fn error_for(&self, dir: &str) -> Option<&str> {
        self.errors.get(dir).map(String::as_str)
    }
}
```

（`RemoteTree::visible_rows` 用之前必须先对根目录调一次
`toggle(&self.root)` 让它进入 `expanded` 状态并触发首次 `readdir`——
根目录不自动展开,这个初始化时机由调用方(`SftpTabState::new`,见
Task 3)负责,`RemoteTree::new` 本身不隐式展开根目录,保持这个类型
"纯数据结构,不做隐式副作用"的边界。）

- [ ] **Step 2: 单测**

同文件末尾 `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str, is_dir: bool) -> RemoteEntry {
        RemoteEntry { path: path.to_string(), name: name.to_string(), is_dir }
    }

    #[test]
    fn toggle_expands_and_requests_on_first_open() {
        let mut tree = RemoteTree::new("/root");
        assert_eq!(tree.toggle("/root"), Some("/root".to_string()));
    }

    #[test]
    fn toggle_collapses_without_requesting() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        assert_eq!(tree.toggle("/root"), None); // 收起
    }

    #[test]
    fn toggle_reopen_with_cache_does_not_request() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/a", "a", false)]));
        tree.toggle("/root"); // 收起
        assert_eq!(tree.toggle("/root"), None); // 重新展开,缓存还在,不重复请求
    }

    #[test]
    fn set_children_sorts_dirs_before_files_then_by_name() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children(
            "/root",
            Ok(vec![
                entry("/root/z.txt", "z.txt", false),
                entry("/root/bdir", "bdir", true),
                entry("/root/a.txt", "a.txt", false),
                entry("/root/adir", "adir", true),
            ]),
        );
        let rows = tree.visible_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["adir", "bdir", "a.txt", "z.txt"]);
    }

    #[test]
    fn set_children_err_recorded_and_does_not_panic_on_visible_rows() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Err("权限拒绝".to_string()));
        assert_eq!(tree.error_for("/root"), Some("权限拒绝"));
        assert!(tree.visible_rows().is_empty()); // 没有子项缓存,可见行为空,不 panic
    }

    #[test]
    fn nested_expansion_shows_grandchildren() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/sub", "sub", true)]));
        tree.toggle("/root/sub");
        tree.set_children("/root/sub", Ok(vec![entry("/root/sub/f", "f", false)]));
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "sub");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "f");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn collapsed_dir_hides_its_children_from_visible_rows() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/sub", "sub", true)]));
        tree.toggle("/root/sub");
        tree.set_children("/root/sub", Ok(vec![entry("/root/sub/f", "f", false)]));
        tree.toggle("/root/sub"); // 收起
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "sub");
    }
}
```

- [ ] **Step 3: 注册子模块**

`crates/dozer-app/src/extensions/ssh.rs` 文件顶部(`use` 语句之前或
之后均可,参照 `extensions/project.rs` 对 `mod links;` 的既有写法)加:

```rust
pub mod sftp;
```

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --lib extensions::ssh::sftp:: -- --test-threads=1 2>&1 | tail -60`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add RemoteTree data model for SFTP remote file browsing

Pure logic, no real SFTP IO yet — toggle/set_children/visible_rows
mirror crate::project::FileTree's shape so both trees can share one
row-rendering function later.

EOF
)"
```

---

### Task 3: `open_sftp_session` 连接函数 + `SftpTabState`/`SftpMessage`

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`

**Interfaces:**
- Consumes: `ssh::handshake`(`ssh.rs:249-284`,阶段 1/2 已有,协议无关
  的握手函数)、`ssh::SshHost`/`ssh::SshError`。
- Produces: `sftp::open_sftp_session(host, password) ->
  Result<russh_sftp::client::SftpSession, ssh::SshError>`、
  `sftp::SftpTabState`、`sftp::Message`(SFTP 专属消息,嵌套在
  `ssh::Message` 里,见 Task 4)。

- [ ] **Step 1: `open_sftp_session`**

```rust
/// 建一条独立 SSH 连接 + 打开 SFTP 子系统 channel。每个 SFTP tab 调用
/// 一次,不复用同一主机终端 tab 的连接(brainstorming 阶段已确认的
/// 决定)。`handle`(`russh::client::Handle`)必须存活到 `SftpSession`
/// 不再使用为止——`into_stream()` 消费了 `channel`,但 `handle` 是
/// 另一个值,调用方(`Workspace::spawn_sftp_tab`,Task 4)必须把
/// `handle` 一起存进某个长期持有的位置,不能让它在这个函数返回后被
/// 提前析构(同阶段 2 终端连接的既有约束)。
pub(crate) async fn open_sftp_session(
    host: &super::SshHost,
    password: Option<String>,
) -> Result<
    (
        russh::client::Handle<super::TestHandler>,
        russh_sftp::client::SftpSession,
    ),
    super::SshError,
> {
    let handle = super::handshake(host, password).await?;
    let channel = handle.channel_open_session().await?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(super::SshError::Russh)?;
    let stream = channel.into_stream();
    let sftp = russh_sftp::client::SftpSession::new(stream)
        .await
        .map_err(|e| super::SshError::Russh(russh::Error::IO(std::io::Error::other(e))))?;
    Ok((handle, sftp))
}
```

（`SshError` 目前(`ssh.rs:173-184`)没有一个能直接装 `russh_sftp` 的
错误类型的变体——上面用 `SshError::Russh(russh::Error::IO(...))` 硬转
一层,写这一步时核实 `russh_sftp` 的错误类型(`SftpResult<T>` 的
`Err` 分支类型,docs.rs 上查到的是某个 `Error` 枚举,不是
`std::io::Error`,如果它没有现成的 `Into<std::io::Error>`,用
`.to_string()` 包一层字符串错误更稳妥——`SshError` 要不要新增一个
`Sftp(String)` 变体,由写这一步时根据 `russh_sftp::error::Error` 的
实际 `Display`/`Into` 实现决定,不强制沿用上面这行"硬转 IO 错误"的
写法,只要类型能编译过、错误文案对用户可读即可)。

- [ ] **Step 2: `SftpTabState`**

```rust
/// 一个 SFTP tab 的完整状态。挂在 `Workspace.sftp_tabs`
/// (`HashMap<host_id, SftpTabState>`,见 Task 4),不是 `SessionTab`
/// ——没有 `TerminalModel`,是"两棵文件树 + 选中态"而不是字节流查看器。
pub struct SftpTabState {
    pub host_id: String,
    pub local_tree: crate::project::FileTree,
    pub remote_tree: RemoteTree,
    pub selected_local: Option<std::path::PathBuf>,
    pub selected_remote: Option<String>,
    pub status: Option<(String, bool)>, // 文案 + 是否为错误(true=红)
    /// SFTP 连接句柄(握手 `Handle` + 会话),`Option` 是因为连接是异步
    /// 建立的——tab 刚打开时 `None`,`Message::Connected` 到达后填入;
    /// 连接失败时永远是 `None`,`status` 落错误文案。
    pub(crate) session: Option<(
        russh::client::Handle<super::TestHandler>,
        russh_sftp::client::SftpSession,
    )>,
}

impl SftpTabState {
    pub fn new(host_id: String, local_root: std::path::PathBuf, remote_root: String) -> Self {
        let mut remote_tree = RemoteTree::new(remote_root);
        remote_tree.toggle(remote_tree.root().to_string().as_str()); // 首次展开根目录
        Self {
            host_id,
            local_tree: crate::project::FileTree::new(local_root),
            remote_tree,
            selected_local: None,
            selected_remote: None,
            status: None,
            session: None,
        }
    }
}
```

（`crate::project::FileTree::new(root)` 构造后是否需要额外调用一次
`refresh`/`reload_from_disk` 才能看到根目录内容,写这一步时核对
`extensions/files.rs` 现有代码构造 `FileTree` 的完整流程——`files.rs`
可能在构造后紧跟着调了一次刷新,`SftpTabState::new` 要照抄那个完整
序列,不能只调 `FileTree::new` 就假设它自动读了根目录。）

- [ ] **Step 3: `Message`(SFTP 专属,嵌套模块)**

每个变体都带 `host_id` 首字段——`ws.sftp_tabs` 是 `HashMap<host_id,
SftpTabState>`,`route()`(Task 6)靠这个字段直接 `get_mut(&host_id)`
定位到具体 tab,不需要在多个同时打开的 SFTP tab 之间"猜"消息属于
哪一个:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    /// 异步连接结果。`Ok` 时触发一次根目录 `readdir`;`Err` 落 `status`
    /// 错误文案。
    Connected(String /* host_id */, Result<(), String>),
    LocalToggle(String /* host_id */, std::path::PathBuf),
    LocalSelect(String /* host_id */, std::path::PathBuf),
    RemoteToggle(String /* host_id */, String /* remote dir */),
    RemoteDirLoaded(String /* host_id */, String /* dir */, Result<Vec<RemoteEntry>, String>),
    RemoteSelect(String /* host_id */, String /* remote path */),
    Upload(String /* host_id */),
    Download(String /* host_id */),
    TransferResult(String /* host_id */, Result<(), String>),
    ContextMenuOpen { host_id: String, is_local: bool, path: String },
    ContextMenuClose,
}
```

（`ContextMenuOpen`/`ContextMenuClose` 提前在这里一并定义好——Task 8
需要它们,提前定义避免 Task 8 再回来改一次这个枚举。`SftpTabState`
Step 2 相应加 `pub context_menu: Option<(bool, String)>` 字段。）

（`Connected` 的 `Result<(), String>` 里,连接成功后真正的
`(Handle, SftpSession)` 怎么从异步任务传回 `Workspace`——`russh::
client::Handle`/`SftpSession` 都不是 `Clone`,不能塞进 `Message` 这种
经 `EventLoopProxy::send_event` 跨线程传递的类型里。写这一步时核实
现状怎么处理"异步任务算出一个不能塞进 `Message` 的值,又要交给
`Workspace`":参照阶段 4 `spawn_ssh_tab` 的既有模式——`Handle` 从来
没有被塞进任何 `Message`,而是在同一个 `handle.spawn(async move {
...})` 任务内部全程持有、只在需要时把**衍生数据**(`bytes`/错误
字符串)经 `Message` 送出去。`open_sftp_session` 返回的 `(Handle,
SftpSession)` 同理必须留在 spawn 出去的那个异步任务内部,`Message::
Connected` 只带"连上了/没连上"这个布尔结果;真正的读写操作
(`RemoteToggle` 触发的 `readdir`、`Upload`/`Download`)都要在**同一个
持有 session 的异步任务**里通过某种任务内部的命令通道(类似阶段 2
`SshOut`/`rx_out` 的既有模式)完成,不是"每次操作现查 `SftpTabState.
session` 字段、直接 `.await` 一个存在结构体里的值"——**这意味着
`SftpTabState.session` 字段的设计(Step 2 那版)是错的,不能这样做,
写这一步时按下面 Task 4 的修正版重新设计连接生命周期管理,不要机械
照抄 Step 2 的 `session: Option<(Handle, SftpSession)>` 字段。**)

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app --lib 2>&1 | tail -40`
Expected: `sftp.rs` 内部可能因为 Step 3 末尾提到的设计问题报生命周期/
`Send` 相关错误(`SftpSession`/`Handle` 不是 `'static`+`Send` 友好到
可以随便存进一个要被 `iced` `Element` 借用的结构体里)——这是预期的
中间状态,Task 4 会重新设计连接持有方式并修正这里。这一步只需要确认
`RemoteTree`/`Message` 枚举本身(不含 `SftpTabState.session` 那个
问题字段)能编译。如果 `SftpTabState` 因为 `session` 字段报错卡住了
整个文件编译,允许暂时把 `session` 字段类型换成
`Option<mpsc::UnboundedSender<SftpCmd>>`(Task 4 的正确设计,提前
在这里用上也可以,两个 Task 顺序对调不影响最终结果,写计划时按哪个
顺序更符合"每个 Task 都能独立编译通过"这条硬要求来定)。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add open_sftp_session and SFTP tab message/state shapes

EOF
)"
```

---

### Task 4: SFTP 连接生命周期 —— 命令通道模式(镜像阶段 2 `SshOut`)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`(修正
  `SftpTabState`,新增 `SftpCmd`)
- Modify: `crates/dozer-app/src/workspace.rs`(新增
  `Workspace.sftp_tabs: HashMap<String, sftp::SftpTabState>` +
  `spawn_sftp_tab`)

**背景(给实现者)**:Task 3 末尾发现的问题——`SftpSession`/
`russh::client::Handle` 不能被塞进 `SftpTabState`(一个要被 iced 渲染
层借用、活在主线程 `Workspace` 上的结构体),因为真正的 SFTP 读写都是
`.await` 调用,不能在 `view()`/`update()` 这种同步上下文里直接跑。
阶段 2 的 `spawn_ssh_tab` 已经解决过一模一样的问题(终端读写也是异步
的)——方案是:握手 + channel 全程留在一个 `handle.spawn` 出去的
异步任务内部,UI 侧只留一个 `mpsc::UnboundedSender` 用来发"命令"给
这个任务,任务收到命令后执行真正的 IO、把结果通过 `EventLoopProxy`
回灌成 `Message`。SFTP 照搬这个模式。

**Interfaces:**
- Produces: `sftp::SftpCmd`(readdir/read/write/mkdir 四种命令)、
  `Workspace::spawn_sftp_tab(&mut self, io: &ShellIo, host_id: String)`、
  `Workspace.sftp_tabs: HashMap<String, sftp::SftpTabState>`。

- [ ] **Step 1: 修正 `SftpTabState`,新增 `SftpCmd`**

`sftp.rs` 里把 Task 3 Step 2 的 `SftpTabState.session` 字段换成:

```rust
/// SFTP 任务内部的命令(UI 侧通过这个通道请求 IO,不直接持有
/// `SftpSession`)。
pub(crate) enum SftpCmd {
    ReadDir(String /* dir */),
    Upload { local: std::path::PathBuf, remote_dir: String },
    Download { remote: String, local_dir: std::path::PathBuf },
}

pub struct SftpTabState {
    pub host_id: String,
    pub local_tree: crate::project::FileTree,
    pub remote_tree: RemoteTree,
    pub selected_local: Option<std::path::PathBuf>,
    pub selected_remote: Option<String>,
    pub status: Option<(String, bool)>,
    /// 右键菜单展开态:`true` = 本地行触发,`false` = 远程行触发,配上
    /// 那一行的路径。`None` = 未展开(Task 8)。
    pub context_menu: Option<(bool, String)>,
    /// 命令通道:`None` = 连接还没建好(或已断开)。`Some` 时 UI 侧发
    /// `SftpCmd`,真正的 IO 在 `Workspace::spawn_sftp_tab` 起的异步
    /// 任务里跑。
    pub(crate) cmd_tx: Option<tokio::sync::mpsc::UnboundedSender<SftpCmd>>,
}

impl SftpTabState {
    pub fn new(host_id: String, local_root: std::path::PathBuf, remote_root: String) -> Self {
        let mut remote_tree = RemoteTree::new(remote_root.clone());
        remote_tree.toggle(&remote_root);
        Self {
            host_id,
            local_tree: crate::project::FileTree::new(local_root),
            remote_tree,
            context_menu: None,
            selected_local: None,
            selected_remote: None,
            status: None,
            cmd_tx: None,
        }
    }
}
```

`Message::Connected` 之外,`open_sftp_session` 的返回值不再直接暴露给
调用方类型签名——它只在 Step 2 新增的 `Workspace::spawn_sftp_tab`
内部使用。

- [ ] **Step 2: `Workspace` 新增字段 + `spawn_sftp_tab`**

`workspace.rs` 里 `Workspace` 结构体(`ssh_tabs`/`ssh_active` 那两个
字段,阶段 4 新增的)之后加:

```rust
    /// SFTP tab 状态(阶段 3),按 host_id 去重——同一主机同时只有一个
    /// SFTP tab 有意义(见 spec 的既有论证)。
    pub(crate) sftp_tabs: HashMap<String, ssh::sftp::SftpTabState>,
```

构造点(`ssh_tabs: Vec::new(), ssh_active: None,` 那两行)之后加
`sftp_tabs: HashMap::new(),`。

新增方法(放在 `spawn_ssh_tab` 附近):

```rust
    /// 打开一个 SFTP tab:建立独立 SSH 连接 + SFTP 子系统,起一个持有
    /// 连接、监听命令通道的异步任务(镜像 `spawn_ssh_tab` 的整体结构)。
    pub(crate) fn spawn_sftp_tab(&mut self, io: &ShellIo, host_id: String) {
        let Some(host) = self.ssh.hosts().iter().find(|h| h.id == host_id).cloned() else {
            return;
        };
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let project_root = std::path::PathBuf::from(&project.path);
        let password = ssh::keyring_password(project_id, &host_id);
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<ssh::sftp::SftpCmd>();
        let proxy = io.proxy.clone();

        // 先在本地(同步)插入一个占位 tab 状态,连接结果异步回填——
        // 这样"点文件传输图标"能立刻看到一个 tab 出现(带 loading 态),
        // 不用等 10 秒握手超时才有任何 UI 反馈。
        self.sftp_tabs.insert(
            host_id.clone(),
            ssh::sftp::SftpTabState::new(host_id.clone(), project_root, "~".to_string()),
        );

        let host_id_for_task = host_id.clone();
        io.handle.spawn(async move {
            let handshake_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ssh::sftp::open_sftp_session(&host, password),
            )
            .await;
            let (handle, sftp) = match handshake_result {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                        ssh::sftp::Message::Connected(host_id_for_task, Err(e.to_string())),
                    )));
                    return;
                }
                Err(_) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                        ssh::sftp::Message::Connected(
                            host_id_for_task,
                            Err("连接超时(10 秒)".to_string()),
                        ),
                    )));
                    return;
                }
            };
            let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                ssh::sftp::Message::Connected(host_id_for_task.clone(), Ok(())),
            )));
            // `handle` 必须留在这个任务作用域内到循环结束——同阶段 2
            // 终端连接的既有约束,理由一样(不能提前析构掉 SSH 连接本身)。
            let _handle_keepalive = handle;
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    ssh::sftp::SftpCmd::ReadDir(dir) => {
                        let result = sftp
                            .read_dir(dir.clone())
                            .await
                            .map(|read_dir| {
                                read_dir
                                    .map(|e| ssh::sftp::RemoteEntry {
                                        path: e.path(),
                                        name: e.file_name(),
                                        is_dir: e.file_type().is_dir(),
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                            ssh::sftp::Message::RemoteDirLoaded(
                                host_id_for_task.clone(),
                                dir,
                                result,
                            ),
                        )));
                    }
                    ssh::sftp::SftpCmd::Upload { local, remote_dir } => {
                        let result = ssh::sftp::upload(&sftp, &local, &remote_dir).await;
                        let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                            ssh::sftp::Message::TransferResult(host_id_for_task.clone(), result),
                        )));
                    }
                    ssh::sftp::SftpCmd::Download { remote, local_dir } => {
                        let result = ssh::sftp::download(&sftp, &remote, &local_dir).await;
                        let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                            ssh::sftp::Message::TransferResult(host_id_for_task.clone(), result),
                        )));
                    }
                }
            }
            // cmd_tx 全部 drop(tab 被关闭)→ 循环退出 → handle/sftp 析构 →
            // 连接关闭。
        });

        if let Some(state) = self.sftp_tabs.get_mut(&host_id) {
            state.cmd_tx = Some(cmd_tx);
        }
    }
```

（`ssh::Message::Sftp(ssh::sftp::Message)` 这个嵌套变体是 Task 5 才
定义——写这一步时如果 Task 5 还没做,先允许这处编译错误,或者调整
执行顺序先做 Task 5 的 `Message` 嵌套再回来写这个函数,两个 Task
之间有循环依赖关系,写计划阶段按怎么分批提交更自然来决定顺序,不强制
死板按 Task 编号顺序执行。`ssh::sftp::upload`/`download` 两个自由函数
是 Task 6 定义,同理。）

- [ ] **Step 3: 编译 + 提交**

这个 Task 大概率不能独立编译通过(依赖 Task 5/6 还没写的符号),写
计划阶段如果发现"每个 Task 必须能独立编译"这条硬约束和"`spawn_sftp_
tab`/`Message::Sftp`/`upload`/`download` 互相循环依赖"冲突,把 Task
4/5/6 合并成一个更大的 Task 一次性提交(bite-sized 步骤仍然按 Step
拆,只是最终编译确认/commit 挪到合并后的末尾)——**这是一个例外情况,
本计划明确允许**,因为这几个符号之间的相互引用在 Rust 里没有办法
干净地拆成互相独立可编译的中间态(不像前面纯新增类型/纯替换消息名那
种可以每步都编译通过的改动)。

---

### Task 5: `ssh::Message::Sftp` 嵌套变体 + 内核路由

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(`Message` 枚举加
  `Sftp(sftp::Message)` 变体,`update()` 加对应分支)
- Modify: `crates/dozer-app/src/app.rs`(阶段 4 里
  `Message::Ssh(ssh::Message::OpenSshTab(_, ssh::SshTabKind::Sftp)) =>
  {}` 那个空分支接上真实逻辑;新增 `Message::Ssh(ssh::Message::Sftp(..))`
  的路由)

- [ ] **Step 1: `ssh::Message` 加 `Sftp` 变体**

```rust
    /// SFTP tab 内部交互,嵌套消息(见 `sftp::Message`)。内核按 host_id
    /// 路由到对应 `ws.sftp_tabs` 条目,不会转发到这个模块自己的
    /// `update()`(同 `OpenSshTab`/`CloseSshTab` 的既有拦截模式——大部分
    /// `sftp::Message` 变体要么需要 `&mut Workspace`(发起异步 IO 命令),
    /// 要么需要直接改 `ws.sftp_tabs`,`ssh::update` 只有 `&mut ws.ssh`
    /// 够不到)。
    Sftp(sftp::Message),
```

`update()` 里加穷尽分支(同 `OpenSshTab` 那种"到不了,写出来只是为了
穷尽匹配"):

```rust
        Message::Sftp(_) => {}
```

- [ ] **Step 2: `app.rs` 接上 `OpenSshTab(_, Sftp)` 真实逻辑**

阶段 4 Task 13 留的空分支:

```rust
-            Message::Ssh(ssh::Message::OpenSshTab(_, ssh::SshTabKind::Sftp)) => {}
+            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Sftp)) => {
+                self.with_focused_project(|ws, io| {
+                    if ws.sftp_tabs.contains_key(&host_id) {
+                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
+                    } else {
+                        ws.spawn_sftp_tab(io, host_id.clone());
+                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
+                    }
+                });
+            }
```

（`select_ssh_tab`——阶段 4 定义的方法——目前只检查 `ws.ssh_tabs`
是否存在对应身份,SFTP 种类的"存在性"来源是 `ws.sftp_tabs`,不是
`ws.ssh_tabs`。写这一步时回到 `workspace.rs::select_ssh_tab`(阶段 4
Task 7),把它的存在性检查扩展成按 `kind` 分流:`Terminal` 检查
`ssh_tabs`,`Sftp` 检查 `sftp_tabs`;`ssh_active` 字段本身
(`Option<(String, SshTabKind)>`)不用改,已经足够表达"当前显示哪个
tab、什么种类"。）

`Message::Ssh(ssh::Message::CloseSshTab(host_id, ssh::SshTabKind::Sftp))`
分支(阶段 4 Task 13 那个 `if kind == Terminal` 判断,现在 `Sftp` 分支
也要处理):

```rust
            Message::Ssh(ssh::Message::CloseSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, io| match kind {
                    ssh::SshTabKind::Terminal => ws.close_ssh_tab(io, &host_id, kind),
                    ssh::SshTabKind::Sftp => {
                        ws.sftp_tabs.remove(&host_id);
                        if ws.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k))
                            == Some((host_id.as_str(), ssh::SshTabKind::Sftp))
                        {
                            ws.ssh_active = None; // 简化处理:关掉 SFTP tab 后不自动
                                                   // 切到其它 tab,和终端 tab 关闭后的
                                                   // "切到剩下第一个"逻辑不强行统一,
                                                   // 因为 ssh_tabs/sftp_tabs 是两个不同
                                                   // 集合,统一切换逻辑收益不大,YAGNI。
                        }
                    }
                });
            }
```

`Message::Ssh(ssh::Message::Sftp(msg))` 路由(新增分支,放在
`OpenSshTab`/`CloseSshTab`/`SelectSshTab` 几条拦截分支附近):

```rust
            Message::Ssh(ssh::Message::Sftp(msg)) => {
                self.with_focused_project(|ws, io| {
                    ssh::sftp::route(ws, io, msg);
                });
            }
```

（把具体的 `match msg { ... }` 逻辑放进 `sftp::route(ws: &mut
Workspace, io: &ShellIo, msg: sftp::Message)` 这个自由函数里,而不是
直接摊在 `app.rs` 的大 `match` 里——`sftp::Message` 有 7 个变体,内容
又都要操作 `ws.sftp_tabs`,摊平会让 `app.rs` 那个已经很大的 `match`
更难读;`sftp::route` 定义在 Task 6。）

- [ ] **Step 3: 编译 + Commit**

同 Task 4 Step 3 的说明:这几个 Task(4/5/6)之间有真实的相互引用,
按"最终整体能编译"为准,不强求每个 Task 独立编译。写完 Task 6 后
一并跑:

Run: `cargo build -p dozer-app 2>&1 | tail -80`

```bash
git add crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions/ssh/sftp.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): wire SFTP tab lifecycle — spawn_sftp_tab, command channel, kernel routing

EOF
)"
```

---

### Task 6: `sftp::route` —— readdir/上传/下载 消息处理

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`(新增
  `route`/`upload`/`download` 函数)

**Interfaces:**
- Consumes: `Workspace.sftp_tabs`(Task 4)、`SftpCmd`(Task 4)。
- Produces: `sftp::route(ws: &mut Workspace, io: &ShellIo, msg:
  sftp::Message)`、`sftp::upload(sftp: &SftpSession, local: &Path,
  remote_dir: &str) -> Result<(), String>`、`sftp::download(...)`
  (对称)。

- [ ] **Step 1: `route`**

每个 `sftp::Message` 变体都带 `host_id`(Task 3 已按这个形状定义好),
`route()` 里每个分支都是"`ws.sftp_tabs.get_mut(&host_id)` 精确定位到
具体 tab,再改字段/发 `SftpCmd`"这个统一模式,不需要猜测消息属于哪个
tab:

```rust
pub(crate) fn route(ws: &mut crate::workspace::Workspace, _io: &crate::workspace::ShellIo, msg: Message) {
    match msg {
        Message::Connected(host_id, result) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else { return; };
            match result {
                // 连接建好后,cmd_tx 已经由 spawn_sftp_tab 结尾同步设置好
                // (它在异步任务 spawn 之后立刻做的,不需要等 Connected
                // 消息才设置——见 Task 4 Step 2 的 spawn_sftp_tab 实现,
                // cmd_tx 是提前建好传给任务的,不是任务算出来的)。这里
                // 只需要触发一次根目录 readdir。
                Ok(()) => {
                    if let Some(tx) = &state.cmd_tx {
                        let _ = tx.send(SftpCmd::ReadDir(state.remote_tree.root().to_string()));
                    }
                }
                Err(e) => {
                    state.status = Some((format!("SFTP 连接失败: {e}"), true));
                }
            }
        }
        Message::LocalToggle(host_id, dir) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.local_tree.toggle(&dir);
            }
        }
        Message::LocalSelect(host_id, path) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.selected_local = Some(path);
            }
        }
        Message::RemoteToggle(host_id, dir) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else { return; };
            if let Some(need_fetch) = state.remote_tree.toggle(&dir)
                && let Some(tx) = &state.cmd_tx
            {
                let _ = tx.send(SftpCmd::ReadDir(need_fetch));
            }
        }
        Message::RemoteDirLoaded(host_id, dir, result) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.remote_tree.set_children(&dir, result);
            }
        }
        Message::RemoteSelect(host_id, path) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.selected_remote = Some(path);
            }
        }
        Message::Upload(host_id) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else { return; };
            let Some(local) = state.selected_local.clone() else { return; };
            // 上传目标目录:远程树根目录(v1 简化处理,不支持"上传到
            // 当前展开的子目录"——右键菜单本身触发在本地行上,不天然
            // 带一个"目标是哪个远程目录"的选择;真要支持上传到子目录,
            // 需要额外在远程侧维护一个"当前高亮目录"概念,不在本阶段
            // 范围,YAGNI)。
            let remote_dir = state.remote_tree.root().to_string();
            let Some(tx) = &state.cmd_tx else { return; };
            let _ = tx.send(SftpCmd::Upload { local, remote_dir });
            state.status = Some(("正在上传…".to_string(), false));
        }
        Message::Download(host_id) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else { return; };
            let Some(remote) = state.selected_remote.clone() else { return; };
            let local_dir = state.local_tree.root().to_path_buf();
            let Some(tx) = &state.cmd_tx else { return; };
            let _ = tx.send(SftpCmd::Download { remote, local_dir });
            state.status = Some(("正在下载…".to_string(), false));
        }
        Message::TransferResult(host_id, result) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else { return; };
            state.status = Some(match result {
                Ok(()) => ("传输完成".to_string(), false),
                Err(e) => (format!("传输失败: {e}"), true),
            });
            // 上传成功后重新拉一次远程根目录(v1 上传目标固定是根目录,
            // 见 Upload 分支的既有简化),让新文件出现在远程树里;下载
            // 成功后本地树用 FileTree 现有的刷新方法重读根目录,让新
            // 文件出现在本地树里。两边都只刷新根目录这一层,不是整棵
            // 树重新拉取。
            if result.is_ok() {
                if let Some(tx) = &state.cmd_tx {
                    let _ = tx.send(SftpCmd::ReadDir(state.remote_tree.root().to_string()));
                }
                state.local_tree.reload_from_disk();
            }
        }
        Message::ContextMenuOpen { host_id, is_local, path } => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else { return; };
            if is_local {
                state.selected_local = Some(std::path::PathBuf::from(&path));
            } else {
                state.selected_remote = Some(path.clone());
            }
            state.context_menu = Some((is_local, path));
        }
        Message::ContextMenuClose => {
            for state in ws.sftp_tabs.values_mut() {
                state.context_menu = None;
            }
        }
    }
}
```

（`FileTree::reload_from_disk()` 的确切签名/是否需要参数,写这一步时
对照 `crate::project::FileTree` 现有 API 核实——`project.rs:131` 已经
列出这个方法,大概率是 `&mut self` 无参,直接调用即可。）

- [ ] **Step 2: `upload`/`download` 自由函数**

```rust
/// 上传本地文件到远程目录。目录上传:递归遍历本地目录逐个文件调用,
/// 远程侧对应子目录不存在时先 `create_dir`(按需创建目标路径,不是
/// 独立的 mkdir 入口——见 Global Constraints 的既有界限)。
pub(crate) async fn upload(
    sftp: &russh_sftp::client::SftpSession,
    local: &std::path::Path,
    remote_dir: &str,
) -> Result<(), String> {
    if local.is_dir() {
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| "无效的本地路径".to_string())?;
        let target_dir = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
        sftp.create_dir(&target_dir).await.map_err(|e| e.to_string())?;
        let entries = std::fs::read_dir(local).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            Box::pin(upload(sftp, &entry.path(), &target_dir)).await?;
        }
        Ok(())
    } else {
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| "无效的本地路径".to_string())?;
        let target = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
        let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
        sftp.write(target, &bytes).await.map_err(|e| e.to_string())
    }
}

/// 下载远程文件到本地目录(目标目录必须已存在——本地一侧不做隐式
/// mkdir,`local_dir` 来自 `FileTree.root()`,恒存在)。目录下载走
/// `read_dir` 递归,逻辑与 `upload` 对称。
pub(crate) async fn download(
    sftp: &russh_sftp::client::SftpSession,
    remote: &str,
    local_dir: &std::path::Path,
) -> Result<(), String> {
    let name = remote.rsplit('/').next().unwrap_or(remote);
    let metadata = sftp.metadata(remote).await.map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        let target_dir = local_dir.join(name);
        std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
        let entries = sftp.read_dir(remote).await.map_err(|e| e.to_string())?;
        for entry in entries {
            Box::pin(download(sftp, &entry.path(), &target_dir)).await?;
        }
        Ok(())
    } else {
        let bytes = sftp.read(remote).await.map_err(|e| e.to_string())?;
        std::fs::write(local_dir.join(name), bytes).map_err(|e| e.to_string())
    }
}
```

（`Metadata::is_dir()` 的具体方法名写这一步时对照 `russh_sftp` 实际
`Metadata` 类型核实——docs.rs 页面没有列全它的字段/方法,大概率跟
`std::fs::Metadata` 同名(`is_dir`/`is_file`),如果不是,改成读取
`FileType`-等价的信息,不影响整体递归下载的结构。）

- [ ] **Step 3: 编译 + 单测 + Commit**

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: Task 4/5/6 合起来应该能让整个 crate 编译通过(除了 Task 7
还没接的渲染部分——`SshTabKind::Sftp` 目前只在 `app.rs` 的 kernel
match 和 `Workspace` 数据层被处理,渲染函数 `ssh_terminal_pane`/
`ssh_tab_bar` 阶段 4 版本还不认识 SFTP 种类,Task 7 补上)。

`upload`/`download` 的路径拼接逻辑(`{remote_dir}/{name}` 这类字符串
处理)抽出来写纯函数单测(不需要真实 SFTP 连接,只测字符串拼接边界
情况——远程目录以 `/` 结尾/不结尾、文件名含空格等,同 spec"测试策略"
一节的既有要求)。

```bash
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): implement SFTP route() message handling and recursive upload/download

EOF
)"
```

---

### Task 7: 渲染 —— SFTP tab 接入 `ssh_tab_bar`/`ssh_terminal_pane`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(阶段 4 的 `ssh_tab_bar`/
  `ssh_terminal_pane`,扩展成同时认识 `ssh_tabs`(终端)和
  `sftp_tabs`(SFTP))
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`(新增
  `tree_column`/`sftp_pane_view` 渲染函数)

**Interfaces:**
- Consumes: 阶段 4 `ssh_tab_bar`/`ssh_terminal_pane`(要修改,不是新增)、
  Task 2 `RemoteTree::visible_rows`、`crate::project::TreeRow`。

- [ ] **Step 1: `ssh_tab_bar` 认识两种 tab**

阶段 4 版本(`docs/superpowers/plans/2026-08-13-ssh-panel-phase4.md`
Task 14)只遍历 `ws.ssh_tabs`。扩展成:

```rust
fn ssh_tab_bar<'a>(
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut bar = row![].spacing(2);
    for tab in &ws.ssh_tabs {
        // 终端 tab,渲染逻辑不变(阶段 4 已有代码原样保留)。
        // ...(同阶段 4 Task 14 的实现)
    }
    for (host_id, state) in &ws.sftp_tabs {
        let is_active = ws.ssh_active.as_ref()
            == Some(&(host_id.clone(), ssh::SshTabKind::Sftp));
        let label = ws
            .ssh
            .hosts()
            .iter()
            .find(|h| &h.id == host_id)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| host_id.clone());
        let content = row![
            icons::view(icons::IconKind::FolderSync, crate::theme::icon_size::row(), theme::color::DIM),
            text(label).size(theme::font::caption()),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center);
        let (select, close) = tabs::tab_core(
            content.into(),
            crate::theme::icon_size::row(),
            theme::color::DIM,
            true,
            Message::Ssh(ssh::Message::SelectSshTab(host_id.clone(), ssh::SshTabKind::Sftp)),
            Message::Ssh(ssh::Message::CloseSshTab(host_id.clone(), ssh::SshTabKind::Sftp)),
            |_| Message::Noop,
            |_| Message::Noop,
        );
        let bg = if is_active { theme::color::CARD } else { theme::color::BG };
        bar = bar.push(
            container(row![select, close].align_y(iced_widget::core::Alignment::Center))
                .padding([6, 10])
                .style(move |_t: &iced_widget::Theme| container::Style {
                    background: Some(bg.into()),
                    ..container::Style::default()
                }),
        );
        let _ = state; // 阶段 4 遗留写法保留 tab 状态引用,当前只用 host_id 渲染标签
    }
    bar.into()
}
```

- [ ] **Step 2: `ssh_terminal_pane` 按 `ssh_active` 的种类分流内容区**

```rust
fn ssh_terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws.ssh_active.as_ref() {
            Some((host_id, ssh::SshTabKind::Terminal)) => {
                let tab = ws.ssh_tabs.iter().find(|t| {
                    t.info.id.strip_prefix("ssh:") == Some(host_id.as_str())
                });
                match tab {
                    Some(tab) => term_view::view(
                        &tab.model,
                        keyboard_term_target(app.left_view, app.active_zone) == TermTarget::SshPanel,
                        TermTarget::SshPanel,
                    )
                    .into(),
                    None => ssh_empty_state(),
                }
            }
            Some((host_id, ssh::SshTabKind::Sftp)) => {
                match ws.sftp_tabs.get(host_id) {
                    Some(state) => ssh::sftp::sftp_pane_view(state).map(|m| {
                        Message::Ssh(ssh::Message::Sftp(m))
                    }),
                    None => ssh_empty_state(),
                }
            }
            None => ssh_empty_state(),
        };
    container(column![ssh_tab_bar(ws), body].height(Length::Fill))
        .width(width)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BG.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

fn ssh_empty_state<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        text("点主机卡片的终端/文件传输图标开始")
            .size(theme::font::body())
            .color(theme::color::DIM),
    )
    .padding(20)
    .into()
}
```

（阶段 4 原本内联在 `ssh_terminal_pane` 里的空态文案,这一步抽成
`ssh_empty_state()` 复用,三个分支——终端 tab 消失/SFTP tab 消失/
完全没打开——共用同一段文案,不需要分别措辞。`sftp_pane_view` 返回
`Element<sftp::Message,...>`,`.map(...)` 转成顶层 `Message`——这个
写法要求 `sftp::Message` 是 `Clone`(Task 3 已经 `#[derive(Debug,
Clone)]`,满足)。）

- [ ] **Step 3: `sftp::sftp_pane_view`/`tree_column`**

`sftp.rs` 新增:

```rust
/// 精简版行渲染:展开箭头 + 文件夹/文件图标 + 名称。不带 git 状态染色/
/// 重命名编辑态/右键菜单(那些是 Files 面板自己的功能),本地/远程两侧
/// 共用同一份实现。`on_toggle`(仅目录行触发)/`on_select` 由调用方
/// 传入,决定是发 `LocalToggle`/`LocalSelect` 还是
/// `RemoteToggle`/`RemoteSelect`。
fn tree_column<'a>(
    title: &'a str,
    rows: Vec<crate::project::TreeRow>,
    selected: Option<&std::path::Path>,
    on_toggle: impl Fn(std::path::PathBuf) -> Message + 'a,
    on_select: impl Fn(std::path::PathBuf) -> Message + 'a,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    use iced_widget::{MouseArea, column, container, row, text};
    let mut col = column![
        text(title).size(crate::theme::font::subtitle()).color(crate::theme::color::CREAM)
    ]
    .spacing(4);
    for r in rows {
        let indent = "  ".repeat(r.depth);
        let icon = if r.is_dir {
            if r.expanded { crate::icons::IconKind::FolderOpen } else { crate::icons::IconKind::Folder }
        } else {
            crate::icons::IconKind::FileGeneric
        };
        let is_selected = selected == Some(r.path.as_path());
        let path_for_toggle = r.path.clone();
        let path_for_select = r.path.clone();
        let label = row![
            text(indent),
            crate::icons::view(icon, crate::theme::icon_size::row(), crate::theme::color::DIM),
            text(r.name.clone())
                .size(crate::theme::font::body())
                .color(if is_selected { crate::theme::color::CREAM } else { crate::theme::color::DIM }),
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center);
        let row_el = MouseArea::new(container(label).width(iced_widget::core::Length::Fill))
            .on_press(if r.is_dir { on_toggle(path_for_toggle) } else { on_select(path_for_select) })
            .into();
        col = col.push(row_el);
    }
    container(col).padding(8).width(iced_widget::core::Length::FillPortion(1)).into()
}

/// SFTP tab 主视图:左右两栏文件树。
pub fn sftp_pane_view<'a>(
    state: &'a SftpTabState,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    use iced_widget::row;
    let local_rows = state.local_tree.visible_rows();
    let remote_rows = state.remote_tree.visible_rows();
    let host_id = state.host_id.clone();
    let host_id2 = state.host_id.clone();
    row![
        tree_column(
            "本地机器项目文件树",
            local_rows,
            state.selected_local.as_deref(),
            move |p| Message::LocalToggle(host_id.clone(), p),
            move |p| Message::LocalSelect(host_id2.clone(), p),
        ),
        tree_column(
            "远程主机文件树",
            remote_rows,
            state.selected_remote.as_deref().map(std::path::Path::new),
            {
                let h = state.host_id.clone();
                move |p| Message::RemoteToggle(h.clone(), p.to_string_lossy().into_owned())
            },
            {
                let h = state.host_id.clone();
                move |p| Message::RemoteSelect(h.clone(), p.to_string_lossy().into_owned())
            },
        ),
    ]
    .into()
}
```

（这一步的签名细节——`Message::LocalToggle(host_id, path)` 这种带
`host_id` 前缀的变体形状,要求 Task 3 的 `sftp::Message` 定义已经按
Task 6 Step 1 的提醒改成了六个变体全部带 `host_id` 首字段;`tree_
column` 的 `on_toggle`/`on_select` 闭包签名、`FileGeneric`/
`FolderOpen`/`Folder` 这几个 `IconKind` 的具体名字写这一步时对照
`icons.rs` 现有枚举核实拼写一致。远程侧右键菜单(上传/下载入口)这一步
先不接——`MouseArea` 目前只接了左键 `on_press`(切换/选中),右键
菜单的具体接线方式(参照 `extensions/files.rs` 的
`Message::ContextMenuOpen` 既有模式,`on_right_press`)是 Task 8 的
内容,不要在这一步顺手加,保持每个 Task 职责单一。）

- [ ] **Step 4: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: 编译成功。这是本计划里第一次能看到 SFTP tab 真正渲染出来的
节点,建议这一步跑一次 `cargo run -p dozer-app` 手动点开一个 SFTP
tab,确认两栏文件树至少能显示出来(即使还不能右键传输),尽早发现
渲染层的问题,不要攒到 Task 9 才第一次手动验证。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): render SFTP tab as dual local/remote file tree panes

EOF
)"
```

---

### Task 8: 右键菜单 —— 上传/下载入口

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`(`tree_column`
  加右键菜单;`sftp::Message` 加菜单展开态)

- [ ] **Step 1: 核实 Files 面板现有右键菜单的接线方式**

Run: `grep -n "ContextMenuOpen\|on_right_press" crates/dozer-app/src/extensions/files.rs | head -20`

参照这个既有实现的形状(菜单展开态字段 + `on_right_press` 触发 +
弹层渲染 + 点菜单项发具体消息 + 点击外部/选别的项收起),给
`SftpTabState` 加一个类似的菜单展开态字段:

```rust
// SftpTabState 新增字段
pub context_menu: Option<(bool /* true=本地行,false=远程行 */, String /* 该行的路径 */)>,
```

`sftp::Message` 新增:

```rust
    ContextMenuOpen { host_id: String, is_local: bool, path: String },
    ContextMenuClose,
```

- [ ] **Step 2: `tree_column` 加 `on_right_press`,`sftp_pane_view` 弹层**

`tree_column` 的 `MouseArea` 补上右键回调参数(新增一个
`on_context: impl Fn(String) -> Message`),右键时发
`ContextMenuOpen`;`sftp_pane_view` 在 `state.context_menu` 为 `Some`
时,于对应那棵树的对应行下方叠一个两项菜单("上传"仅本地行可点,
"下载"仅远程行可点——按 `is_local` 决定菜单只出现对应那一项还是两项
都出现但只有一项可点,写这一步时按 UI 简洁性决定,倾向"只出现对应那
一项",不出现无意义的禁用态)。

菜单项点击发 `Message::Upload(host_id)`/`Message::Download(host_id)`
(Task 6 已经定义并在 `route()` 里处理了这两个变体的核心逻辑,这一步
只是把触发入口从"没有 UI 能触发"变成"右键菜单能触发")。

- [ ] **Step 3: `route()` 补 `ContextMenuOpen`/`ContextMenuClose` 分支**

```rust
        Message::ContextMenuOpen { host_id, is_local, path } => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.context_menu = Some((is_local, path.clone()));
                if is_local {
                    state.selected_local = Some(std::path::PathBuf::from(path));
                } else {
                    state.selected_remote = Some(path);
                }
            }
        }
        Message::ContextMenuClose => {
            for state in ws.sftp_tabs.values_mut() {
                state.context_menu = None;
            }
        }
```

（`ContextMenuClose` 不带 `host_id`——点击菜单外部关闭时,UI 层不一定
知道是哪个 tab 的菜单开着,直接把所有 `sftp_tabs` 的 `context_menu`
清空是安全的简化处理,同一时刻只会有一个 SFTP tab 显示在屏幕上,不会
误关别的 tab 的有意义状态。）

- [ ] **Step 4: 编译 + Commit**

Run: `cargo build -p dozer-app 2>&1 | tail -80`

```bash
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add right-click upload/download context menu to SFTP tree rows

EOF
)"
```

---

### Task 9: 收尾 —— 全量测试 + clippy + fmt + 人工 GUI 验收

**Files:** 无新增改动(除非清理需要)。

- [ ] **Step 1: 死代码检查**

Run: `cargo build -p dozer-app 2>&1 | grep -i "warning: unused\|warning: never"`
Expected: 空输出。

- [ ] **Step 2: 格式化 + 全量 lint**

Run: `cargo fmt -p dozer-app`
Run: `cargo clippy -p dozer-app --all-targets -- -D warnings 2>&1 | tail -100`
Expected: 无 error。

- [ ] **Step 3: 全量测试**

Run: `cargo test -p dozer-app 2>&1 | tail -100`
Expected: 全部 PASS,含 `extensions::ssh::sftp::tests::*`(Task 2)、
上传/下载路径拼接单测(Task 6)。

- [ ] **Step 4: Commit(如果有清理动作)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
chore(dozer-app): clean up dead code after ssh panel phase 3 sftp implementation

EOF
)"
```

- [ ] **Step 5: 人工 GUI 验收清单**

Run: `cargo run -p dozer-app`,配一台可达的 SSH 主机(本机 `localhost`
最方便测试),核实:

- [ ] 点主机卡片"文件传输"图标:SSH 面板 tab 条新开一个 SFTP tab(与
      同主机可能已经打开的终端 tab 互不影响、可以同时存在)。
- [ ] tab 打开后短暂显示 loading/连接中状态,连接成功后左右两栏文件树
      都能显示内容(左=当前项目根目录,右=远程主机根目录/home)。
- [ ] 本地/远程树都能展开/收起目录,子项正确显示(目录在前、文件在后,
      按名排序)。
- [ ] 选中本地一个文件,右键出现"上传"选项(远程侧不出现"下载"—— 视
      Task 8 的菜单显示规则而定);点击后远程树对应目录能看到新上传的
      文件(可能需要手动收起/展开该目录触发刷新,视 Task 6 的刷新触发
      范围而定)。
- [ ] 选中远程一个文件,右键"下载",本地磁盘对应目录能看到下载下来的
      文件。
- [ ] 断开网络或指向一个不可达主机:SFTP tab 显示明确的连接失败文案,
      不是卡死转圈。
- [ ] 远程目录读取权限拒绝:该目录显示"⚠ 无法读取"文案,不影响其它
      目录正常浏览。
- [ ] 关闭 SFTP tab 再重新点"文件传输"图标打开:是全新连接(不是复用
      上次状态——比如展开态应该重置)。
- [ ] 同时打开同一主机的终端 tab 和 SFTP tab,在两者之间切换,两边
      状态互不干扰(终端里的输入不影响 SFTP 树,反之亦然)。
