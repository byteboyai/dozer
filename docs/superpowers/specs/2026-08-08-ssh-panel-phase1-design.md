# SSH 远程主机面板 · 阶段 1:主机连接管理 + 认证 + 连接测试

**状态:已批准(brainstorming 会话,2026-08-08)**

## 背景

用户提出要加一个基于 SSH 的远程主机管理面板,包含终端和 SFTP 文件管理。
brainstorming 过程中调研了 Rust 生态里可用的 SSH 库:

- [`russh`](https://github.com/Eugeny/russh)(作者 Eugeny,同时是知名 Rust
  终端应用 Tabby 的作者,生产级成熟度):纯 Rust、Tokio 异步、无 C 库依赖,
  支持交互式 PTY 客户端会话,配套 [`russh-sftp`](https://docs.rs/russh-sftp/
  latest/russh_sftp/) 做 SFTP。对比过 `ssh2`(绑定 libssh2 C 库,额外编译
  复杂度)和 `async-ssh2-lite` 类似问题,`russh` 更符合本仓库现有的纯 Rust
  生态取向(同 SQLx 优于绑定 C 库驱动的取舍理由)。
- 关键架构发现:本仓库现有的终端渲染管线(`term_model.rs` 的
  `TerminalModel` 包装 `alacritty_terminal` 的 VTE 解析器)是**协议无关的
  纯字节流处理器**——只关心喂给它的字节,不关心字节来自本地 PTY 还是网络。
  这意味着后续"SSH 终端"阶段大概率可以直接复用现有 `TerminalModel`/
  `term_view.rs` 渲染管线,只需把字节来源从"本地 PTY(经 dozerd)"换成
  "SSH channel(经 `russh`)"——这个发现影响了阶段划分,但阶段 1(本文档)
  本身不涉及终端渲染,只是记录下来供阶段 2 设计时用。

这次范围经用户确认按功能面切成 3 块:① 主机连接管理 + 认证 + 连接测试
(本设计文档范围)、② SSH 终端(复用 `TerminalModel`)、③ SFTP 文件管理
(浏览 + 上传/下载,不做在线编辑)。本文档只覆盖①。

**关键架构决策**(均已与用户确认):

1. **会话生命周期不经 `dozerd` 托管,直接在 `dozer-app` 进程里管理**——
   跟本地终端"关 GUI 会话不断、能 attach 回来"的现状体验不同,SSH 会话
   关 GUI 就断。这是范围裁剪,不是遗漏:阶段 1 只做连接管理/认证/测试,
   还没有"会话"这个东西存在,这条决策主要影响阶段 2(SSH 终端)怎么设计,
   这里先记录下来定调。
2. **面板放在工作区,按项目分组**(不是像首页浏览器那样做成 App 级全局
   面板)——与数据库面板(`docs/superpowers/specs/2026-08-08-database-panel-
   phase1-design.md`)的选择一致,
   两个面板结构因此高度对称。
3. **Host key 验证走标准 SSH 行为**:复用 `~/.ssh/known_hosts`(与系统
   `ssh` 客户端共享同一份信任记录),首次连接新主机弹窗显示指纹让用户
   确认、确认后写入;之后如果 host key 变了(可能中间人攻击)直接拒绝
   连接并明确报错,不静默放行、不提供"跳过验证"的选项。

**排期**:与 `App`/`Workspace` 文件拆分(`feature/app-workspace-split`
分支,进行中)、数据库面板阶段 1(`feature/database-panel-phase1`,进行中)
三者相互独立,唯一交叉点是都要在工作区壳层加一个新的 `LeftView`/`RailButton`
变体——改动量都很小,真发生合并冲突时按"先落地的先合并,后合并的一方
rebase"处理。

## 目标 / 非目标

**目标**:

1. 新建 `crates/dozer-app/src/extensions/ssh.rs`。**不需要 App 级
   `AppState`**——数据库面板需要 `AppState` 是因为"驱动启用/禁用"是个
   全局概念,SSH 没有对应的东西(不存在"多种 SSH 协议可选"),直接一个
   `WorkspaceState` 挂每个 `Workspace` 就够。
2. 数据模型:

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub enum AuthMethod {
       Password,
       PrivateKey { key_path: String },
   }

   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct SshHost {
       pub id: String,
       pub name: String,
       pub host: String,
       pub port: u16,      // 默认 22
       pub username: String,
       pub auth: AuthMethod,
   }
   ```

   **密码/私钥 passphrase 不进 `SshHost`/持久化文件**——单独走 macOS
   Keychain,key 拼法与数据库面板一致:`service = "dozer", account =
   format!("{project_id}:{host_id}")`。`AuthMethod::PrivateKey` 只存
   `key_path`(公开信息,私钥文件本身在磁盘上,不是 Dozer 管理的凭据),
   passphrase(如果私钥文件本身加密)才进 Keychain。
3. 持久化:`.dozer/ssh_hosts.json` 存 `Vec<SshHost>`,读写模式与
   `.dozer/database.json` 完全一致(`serde_json`,读失败/不存在→空列表,
   不 panic)。
4. 面板接线:工作区新增 `LeftView::Ssh` + 对应 `RailButton` 变体 + 新图标
   (Lucide,写计划时选,如 `server` 或 `terminal-square`)。UI 结构对齐
   数据库面板:主机卡片列表(名字 + host:port + 用户名 + 认证方式摘要 +
   "测试连接"/"编辑"/"删除")+ "＋新增主机"表单(name/host/port/username +
   认证方式二选一:密码输入框(`secure`)或私钥文件路径输入框(未来可加
   "浏览…"文件选择,阶段 1 先手输路径)+ 对应密码框)。
5. 连接测试(异步):`russh` 建立 TCP + SSH 握手 + 认证(不开 shell、不跑
   任何命令,只验证"能连上且认证通过"),同样套 5 秒超时,结果经
   `TestConnectionResult(project_id, host_id, Result<(), String>)` 回传
   (**必须带 `project_id`**,理由与数据库面板完全一致——异步结果不能假设
   用户没有切走项目页签)。
6. Host key 验证(核心安全逻辑,阶段 1 的重点):
   - 连接前读取 `~/.ssh/known_hosts`(标准 OpenSSH 格式),查是否已有这台
     主机(按 `host:port` 或裸 `host`,视 OpenSSH 惯例)的记录。
   - 已有记录且匹配→放行。已有记录但不匹配(host key 变了)→**拒绝连接**,
     报错文案明确指出"主机指纹已变化,可能存在中间人攻击风险"(不是笼统
     的"连接失败")。
   - 没有记录(首次连接)→连接测试流程中途暂停,把新主机的指纹(算法 +
     SHA256 摘要,同 `ssh-keyscan`/系统 `ssh` 首次连接弹的那行提示格式)
     通过一条新消息回传给 UI,UI 弹确认框("是否信任这台主机?"+ 指纹),
     用户确认后才继续认证流程、并追加写入 `~/.ssh/known_hosts`;用户取消
     则整个测试判定失败,不写入。

**非目标**(留给阶段 2/3):

- 不做 SSH 终端(交互式 shell 会话)。
- 不做 SFTP 浏览/上传下载。
- 不做端口转发/隧道。
- 不做 SSH agent(`ssh-agent`/`1Password` 等)集成——私钥认证阶段 1 只支持
  直接指定私钥文件路径这一种方式。
- 不做"从 `~/.ssh/config` 导入主机列表"这类便利功能——阶段 1 只有 Dozer
  自己的 `.dozer/ssh_hosts.json`,不读用户已有的 SSH 配置(以后如果需要,
  单独一轮讨论,不在这次范围内擅自加)。

## 架构与数据流

### 1. 文件与模块

- 新建 `crates/dozer-app/src/extensions/ssh.rs`,`extensions.rs` 加
  `pub mod ssh;`(按字母序插入位置写计划时核对——如果数据库面板的
  `pub mod database;` 已经落地,`ssh` 排在它和 `todo` 之间)。
- 结构:`WorkspaceState`(数据源列表见上)+ `Message` + `update` + `view`,
  没有 `AppState`(见"目标"第 1 条)。

### 2. Host key 验证的具体实现

`known_hosts` 文件路径:`dirs`/`directories` crate 或直接
`std::env::var("HOME")` 拼 `~/.ssh/known_hosts`(写计划时核对仓库现有
惯例——`dozer_core::paths` 模块如果已经封装了用户主目录相关路径,优先复用,
不重新发明)。**这个文件不是 Dozer 私有的**——用真实的 OpenSSH
`known_hosts` 格式读写,与用户系统上的 `ssh`/`scp`/`git` over SSH 共享同
一份信任记录,这是刻意的(用户在别处已经信任过的主机,Dozer 里不用重新
确认一遍;反之亦然)。

`russh` 的 SSH 握手回调(`client::Handler` trait 的 `check_server_key` 类
方法)是这套验证逻辑真正接入的地方——写计划时对照 `russh` 当前版本的
`Handler` trait 具体方法名核实(API 可能与 brainstorming 时调研的版本有
出入,以 `cargo doc`/docs.rs 当时锁定的版本为准)。

### 3. 与数据库面板的结构对称性

两个面板(`extensions::database`、`extensions::ssh`)这次是同一批
brainstorming session 定的,刻意保持结构对称(卡片列表 + 表单 + 测试连接
+ 异步结果按 `project_id` 路由这一整套模式),方便以后有第三个"外部连接
类"面板时也能复用同一套心智模型。**不**抽出一个共享的"外部连接管理"
extension 基类/trait——两个面板的数据模型本质不同(数据库是"驱动类型+
连接参数",SSH 是"认证方式+主机指纹"),提前抽象反而增加理解成本,YAGNI。

## 错误处理

- `.dozer/ssh_hosts.json` 读失败→空列表,不崩(同数据库面板)。
- Keychain 读写失败→密码/passphrase 视为空,连接测试报"未找到密码/密钥
  口令"类错误。
- Host key 不匹配→**拒绝连接**,错误文案明确、不与"网络不可达"之类的
  错误混为一谈。
- 私钥文件路径不存在/无法解析(格式错误/需要 passphrase 但没填)→连接
  测试报具体错误(区分"文件不存在"和"私钥需要密码"两种情况,不要笼统的
  "认证失败")。

## 测试策略

- `known_hosts` 解析/匹配逻辑的纯函数单测:给定文件内容 + 主机名,断言
  "已知且匹配"/"已知但不匹配"/"未知"三种分支的判定结果。
- `AuthMethod`/`SshHost` 的序列化往返测试(同数据库面板 `DataSource` 的
  round-trip 测试模式)。
- 真实网络连接(连接测试本身)不适合单测覆盖,人工验收覆盖:对一台已知
  可达的 SSH 主机(比如 `localhost` 如果本机开了 sshd,或者用户提供的
  测试服务器)测试连接成功;对一台 host key 故意改过的主机测试连接应该
  被拒绝并给出明确警告;首次连接新主机应该弹出指纹确认框。

## 依赖变更

新增:
- `russh`(pure Rust SSH 客户端,PTY 交互式会话支持——这次阶段 1 用不上
  PTY,但选它是因为阶段 2 会用到,现在就定下来避免以后换库)。
- `russh-sftp`(阶段 1 实际不用,但作为同一决策的一部分记录在这里——是否
  阶段 1 就加上这个依赖、还是留到阶段 3 再加,写计划时决定,不影响阶段 1
  的功能范围)。
