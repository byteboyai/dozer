# SSH 主机面板 阶段 3:SFTP 文件传输 设计

**状态:已批准(brainstorming 会话,2026-08-13)**

## 背景

`docs/superpowers/specs/2026-08-08-ssh-panel-phase1-design.md` 从一开始就
把 SSH 面板的完整需求定成"主机管理 + 终端 + SFTP 文件管理"三段,并明确
把 SFTP 排到"阶段 3",非目标里反复重申"不做 SFTP 浏览/上传下载"
(phase1/phase2 设计文档均有此条)。截至本文档,阶段 3 从未开工——没有
`russh-sftp` 依赖、没有数据模型、没有消息、没有图标。

用户给了新参考草图:SSH 面板 tab 条里,点主机卡片"文件传输"图标(见
`docs/superpowers/specs/2026-08-13-ssh-panel-phase4-design.md` 新增的
`IconKind`)打开一个新种类的 tab,内容是左右两栏文件树——左栏"本地机器
项目文件树"(当前打开项目的根目录),右栏"远程主机文件树"(该 SSH 主机的
文件系统)。本文档设计这个 tab 的具体实现,依赖阶段 4 已经搭好的
`SshTabKind`/tab 集合/tab 条基础设施。

## 目标 / 非目标

**目标**:

1. `SshTabKind::Sftp` 变体正式启用(阶段 4 已定义枚举,阶段 4 期间不接
   实际内容或不渲染入口——本阶段把内容补上)。点主机卡片"文件传输"图标
   → 在 SSH 面板 tab 条开一个 SFTP tab 并聚焦,tab 前缀图标与终端 tab
   区分(复用阶段 4 新增的文件传输图标)。
2. 每个 SFTP tab 独立新建一条 SSH 连接(不复用同一主机可能已经打开的
   终端 tab 的连接),在这条连接上打开 SFTP 子系统 channel。
3. 左栏本地文件树:复用 `crate::project::FileTree`/`TreeRow`(当前打开
   项目的根目录数据模型,`extensions::files` 面板正在用的同一套),新写
   一个精简版行渲染(展开/收起 + 图标 + 名称,不带 git 状态染色/重命名/
   右键菜单——那些是 Files 面板自己的功能,SFTP 场景不需要)。
4. 右栏远程文件树:新的数据模型 + SFTP `readdir` 驱动,行渲染尽量复用
   目标 3 的精简版行渲染函数(同一套外观,两边视觉一致)。
5. 传输交互:选中本地或远程一侧的文件/目录,右键菜单弹出"上传"(本地→
   远程,仅本地侧文件可选)或"下载"(远程→本地,仅远程侧文件可选)。
6. 远程文件树操作范围:只浏览(展开目录)+ 上传/下载,不做远程
   mkdir/删除/重命名,不做任何文件内容的内联编辑——与阶段 1 设计文档
   "非目标"("不做 SFTP 浏览/上传下载"排除的东西现在要做,但"不做内联
   编辑"这条延续)保持一致的克制口径。

**非目标**:

- 不做目录/文件级的删除、重命名、新建——远程侧的写操作只有"上传覆盖"
  和"下载"两种。
- 不做拖拽传输(右键菜单已覆盖交互需求,iced 跨容器拖拽实现成本高,
  YAGNI)。
- 不做传输进度条/传输队列 UI——上传/下载走一次性异步任务,完成/失败用
  简单的文案提示(见"错误处理"),不做可暂停/可取消的传输管理界面。
- 不做多文件/多选批量传输——一次右键菜单操作只针对当前选中的一个文件
  或目录(目录上传/下载是否递归处理其内容,见"架构与数据流"第 4 节)。
- 不做连接复用(与终端 tab 共享同一条 SSH 连接)——已在 brainstorming
  阶段明确决定"每个 tab 独立新建连接",简单换取隔离性,连接数量增加的
  代价可接受。
- 不改变本地 `crate::project::FileTree` 的既有实现(`toggle`/
  `visible_rows`/`search_rows` 等),只是多一个消费方。

## 架构与数据流

### 1. 新依赖:`russh-sftp`

`Cargo.toml`/`crates/dozer-app/Cargo.toml` 目前没有任何 SFTP 相关依赖
(已核实,阶段 1 设计文档把"选 `russh-sftp` 还是手写协议帧"这个决定明确
留到了写计划阶段,现在决定:选现成的)。新增:

```toml
russh-sftp = "2" # 版本号写计划阶段核实与 russh 0.62 线兼容的具体版本
```

`russh-sftp` 的 SFTP 客户端建立在一条已经认证成功的 `russh::client::
Handle` 打开的 channel 之上(`channel.request_subsystem(true,
"sftp").await` 之后把 channel 交给 `russh_sftp::client::SftpSession::new
(channel_stream)`,具体 API 形状写计划阶段对照 `russh-sftp` 文档核实,
本设计不假设精确到函数签名的细节——上一级"一条 SSH 连接 + 一个 SFTP
子系统 channel"的架构判断是确定的,API 细节留给实现)。

### 2. 每个 SFTP tab 的连接

复用阶段 4 `ssh::handshake()`(`ssh.rs:249-284`,已经是"握手 + 认证,
返回 `Handle`"的独立函数,协议无关,可以被终端和 SFTP 两种用途共用)。
新增:

```rust
// extensions/ssh.rs 或新增 extensions/ssh/sftp.rs 子模块(见"文件结构"
// 决定——阶段 4 的 ssh.rs 已经因为视觉重写+tab 基础设施显著变长,SFTP
// 这块数据模型+驱动逻辑独立成子模块,镜像 extensions/project/links.rs
// 从主文件拆出去的既有先例)。
pub(crate) async fn open_sftp_session(
    host: &SshHost,
    password: Option<String>,
) -> Result<russh_sftp::client::SftpSession, SshError> {
    let handle = handshake(host, password).await?;
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = russh_sftp::client::SftpSession::new(channel.into_stream()).await
        .map_err(|_| SshError::AuthFailed)?; // 精确错误类型写计划阶段对齐 russh-sftp 的 Error 类型
    Ok(sftp)
}
```

(`handle` 同阶段 2 终端连接的既有约束——必须存活到 SFTP session 不再
使用为止,不能提前析构;这条约束沿用阶段 2 设计文档"背景"末尾的既有
论证,不是本阶段新引入的风险。)

### 3. 远程文件树数据模型

```rust
// extensions/ssh/sftp.rs
#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub path: String, // SFTP 路径用 String 不用 PathBuf——远程可能是不同
                       // 操作系统(理论上,虽然目标场景基本是 Linux),避免
                       // PathBuf 的本地路径分隔符语义混进来
    pub name: String,
    pub is_dir: bool,
}

/// 远程文件树状态,镜像 `crate::project::FileTree` 的形状(展开集合 +
/// 子项缓存),但不是同一个类型——远程用异步 SFTP readdir 填充缓存,本地
/// 是同步 `std::fs::read_dir`,两者填充时机/错误处理路数不同,共享同一个
/// 类型会把"本地同步遍历"和"远程异步 IO 结果回填"两种生命周期硬拧在
/// 一起,不如各自独立、渲染层共用同一个精简行渲染函数(见目标 3/4)。
pub struct RemoteTree {
    root: String, // 远程起始目录,通常是 "." 或用户 home,写计划阶段定
    expanded: std::collections::HashSet<String>,
    children: std::collections::HashMap<String, Vec<RemoteEntry>>,
}

impl RemoteTree {
    pub fn new(root: String) -> Self { .. }
    /// 展开/收起某个远程目录。展开且缓存里没有 → 返回 `Some(path)`,
    /// 调用方(`update()`)据此发起一次异步 `readdir`;已经展开或已有
    /// 缓存 → 返回 `None`,不重复请求。
    pub fn toggle(&mut self, dir: &str) -> Option<String> { .. }
    /// 异步 `readdir` 结果回填(成功/失败都要落地,失败时该目录标记成
    /// "空+错误提示",不是无限转圈)。
    pub fn set_children(&mut self, dir: &str, result: Result<Vec<RemoteEntry>, String>) { .. }
    /// 渲染用的可见行,形状对齐 `crate::project::TreeRow`(复用同一个
    /// 行渲染函数需要同构的字段:path/name/depth/is_dir/expanded)。
    pub fn visible_rows(&self) -> Vec<crate::project::TreeRow> { .. }
        // TreeRow.path 是 PathBuf——远程路径塞进 PathBuf 只当"不透明字符串
        // 容器"用(不在远程路径上调用任何 PathBuf 的本地文件系统语义
        // 方法,比如 canonicalize),写计划阶段确认这个复用是否可接受,
        // 不行就让 TreeRow 泛型化或新增一个 RemoteTreeRow 变体——优先
        // 尝试直接复用,减少重复类型。
}
```

### 4. `SftpTabState`:一个 SFTP tab 的完整状态

```rust
// extensions/ssh/sftp.rs
pub struct SftpTabState {
    pub host_id: String,
    pub local_tree: crate::project::FileTree,   // 复用,根目录=当前项目根
    pub remote_tree: RemoteTree,
    pub selected_local: Option<PathBuf>,
    pub selected_remote: Option<String>,
    pub status: Option<(String, bool)>, // 文案 + 是否为错误(true=红色)
}
```

这个状态不属于 `ssh::WorkspaceState`(那个是主机列表/编辑表单的状态),
也不属于 `Workspace.ssh_tabs`(那是终端类 tab 的 `SessionTab` 集合)——
SFTP tab 没有 `TerminalModel`,不是"字节流查看器",是"两棵文件树 + 选中
状态"。新增平行集合(仿照阶段 4 `ssh_tabs`/`ssh_active` 的模式):

```rust
// workspace.rs, Workspace 结构体
pub(crate) sftp_tabs: HashMap<String /* host_id */, sftp::SftpTabState>,
```

(用 `HashMap<host_id, _>` 而不是 `Vec`,因为阶段 4 已经决定"tab 身份 =
(host_id, kind)"、SFTP 种类下同一主机同时只会有一个 tab 有意义——不像
终端 tab 理论上可以同主机开多个终端 session,SFTP 浏览会话按主机去重
更符合直觉,写计划阶段如果发现"同主机开两个独立 SFTP tab"也是合理需求,
再改回 `Vec`,本设计先按"每主机一个 SFTP tab"处理,YAGNI。)

阶段 4 定义的 `ssh_active: Option<(String, SshTabKind)>` 保持不变,
`SshTabKind::Sftp` 命中时,SSH 面板内容区渲染的不是
`term_view::view(...)`,而是新增的 `sftp_pane_view(&sftp_tabs[host_id],
...)`。

### 5. `Message` 新增

```rust
// extensions/ssh.rs::Message,或 extensions/ssh/sftp.rs 自己的子 Message
// (阶段 4 的 OpenSshTab/CloseSshTab/SelectSshTab 已经是 host_id+kind 
// 寻址,可以直接复用;SFTP 内部交互——展开目录/选中文件/上传/下载——
// 新增专属变体,写计划阶段决定挂在 ssh::Message 还是拆一个
// ssh::sftp::Message 嵌套进去,取决于 SFTP 交互消息数量,预估 6-8 个,
// 参考 Project 面板 links 子模块"数据结构+消息都独立"的既有先例,倾向
// 拆一个嵌套 Message)。
pub enum SftpMessage {
    LocalToggle(PathBuf),
    LocalSelect(PathBuf),
    RemoteToggle(String),
    RemoteDirLoaded(String /* host_id */, String /* dir */, Result<Vec<RemoteEntry>, String>),
    RemoteSelect(String),
    Upload,   // 用当前 selected_local 上传到 remote_tree 当前展开的目录
              // (具体"上传到哪个远程目录"的语义——选中远程侧当前高亮的
              // 目录,还是右栏根目录——写计划阶段结合右键菜单交互定,
              // 这条决定不影响架构,只影响一处目标目录计算)
    Download, // 用当前 selected_remote 下载到本地对应目录
    TransferResult(String /* host_id */, Result<(), String>),
}
```

### 6. 传输:右键菜单 + 异步任务

选中文件/目录后的右键菜单复用 Files 面板已有的
`Message::ContextMenuOpen`-类交互模式(不是同一个 `Message` 类型,是同样
的"选中 + 右键弹菜单 + 菜单项发具体消息"这套 UI 模式)。"上传"/"下载"
点击后:

```rust
// 上传:本地文件 → 远程目标目录。异步任务用 handle.spawn,完成后
// emit(SftpMessage::TransferResult(host_id, result))。
async fn upload_file(
    sftp: &russh_sftp::client::SftpSession, // 或按连接生命周期管理方式调整
    local_path: &Path,
    remote_dir: &str,
) -> Result<(), String> {
    // 读本地文件 → sftp.create(remote_path) → 写字节。目录上传:递归
    // 遍历本地目录,逐个文件调用本函数,远程侧对应子目录不存在时先
    // sftp.create_dir(..)(mkdir 只在"上传时按需创建目标路径"这一个
    // 场景里出现,不是"非目标"里排除的那种独立 mkdir 操作入口——两者
    // 不矛盾,写计划阶段在设计里把这条界限写清楚,避免实现者理解成
    // "完全不能调用任何创建目录的 SFTP API")。
}
```

下载同理,方向相反。传输过程中 `status` 字段展示"正在上传 xxx…",完成
后展示结果文案(3-5 秒后自动清除,或点别的操作时清除——写计划阶段按
现有类似"临时状态提示"组件的既有习惯定,比如 `TestStatus` 展示逻辑)。

### 7. 视图布局

```rust
fn sftp_pane_view<'a>(
    state: &'a sftp::SftpTabState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        tree_column("本地机器项目文件树", state.local_tree.visible_rows(), state.selected_local.as_deref(), /* on_toggle */ ..., /* on_select */ ...),
        divider,
        tree_column("远程主机文件树", state.remote_tree.visible_rows(), state.selected_remote.as_deref(), ...),
    ]
}
```

`tree_column` 是目标 3/4 提到的精简行渲染函数,两侧共用同一份实现(参数
化标题文案+数据源+选中态+回调,内部结构镜像 `extensions/files.rs`
现有行渲染的展开箭头+文件夹/文件图标+名称这部分,去掉 git 状态染色/
重命名编辑态/右键菜单——SFTP 场景不需要,右键菜单是外层 `MouseArea`
包一层单独接的,不复用 Files 面板的 `ContextMenuOpen` 具体菜单项集合)。

## 错误处理

- SFTP 连接建立失败(握手/子系统协商失败):tab 打开但内容区展示错误
  文案("SFTP 连接失败: {err}"),不阻塞 SSH 面板其它功能,不影响已经
  打开的终端 tab。
- 远程目录读取失败(权限拒绝等):`RemoteDirLoaded` 的 `Err` 分支——该
  目录在树里标记展开但显示"⚠ 无法读取: {err}",不是无限 loading。
- 上传/下载失败(网络中断、目标路径权限拒绝、磁盘空间不足等):
  `TransferResult` 的 `Err` 分支落进 `status` 展示红色错误文案,不回滚
  已经部分写入的字节(SFTP 传输不是事务性的,同类工具——scp/rsync——
  的既有行为习惯也是"失败了留下部分文件,用户自己清理",不新增回滚
  逻辑)。
- 本地/远程路径在传输过程中被删除/改名(并发场景):传输任务失败,
  按上一条的 `TransferResult(Err(..))` 路径处理,不做特殊检测。

## 测试策略

- `RemoteTree::toggle`/`set_children`/`visible_rows` 的状态迁移:纯逻辑,
  可以完全脱离真实 SFTP 连接单测(用手造的 `RemoteEntry` 列表模拟
  `readdir` 结果)。
- `SftpTabState` 的选中态迁移(`LocalSelect`/`RemoteSelect` 互相独立,
  选中一侧不清空另一侧——上传/下载分别只依赖各自那一侧的选中值)。
- 上传/下载的路径计算(本地文件名拼到远程目标目录、远程文件名拼到本地
  目标目录)是纯函数,单测覆盖路径拼接边界情况(远程目录以 `/` 结尾/
  不结尾、本地路径含空格等)。
- 真实 SFTP 读写(`open_sftp_session`/`upload_file`/`download_file`)不做
  自动化集成测试(需要真实可达的 SSH 服务器,同阶段 1/2 现有测试策略
  ——那两个阶段的 russh 握手/终端 pump 也没有集成测试,只测纯逻辑
  部分),人工 GUI 验收覆盖。
- 人工 GUI 验收清单(写进实现计划):点"文件传输"图标开一个 SFTP tab,
  与该主机可能已经打开的终端 tab 互不影响;本地树能展开/收起、显示
  当前项目文件;远程树能展开/收起、显示远程文件系统;选中本地文件右键
  "上传"后远程树刷新能看到新文件;选中远程文件右键"下载"后本地磁盘上
  能看到文件;断开网络/主机不可达时连接失败有明确错误提示,不是卡死
  转圈;关闭 SFTP tab 后再重新打开,建立的是全新连接(不复用上次状态)。

## 依赖变更

新增 `russh-sftp`(版本写计划阶段核实,对齐现有 `russh = "0.62.5"`)。
