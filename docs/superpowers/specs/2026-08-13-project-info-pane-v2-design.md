# 项目信息(Project)面板 v2 设计

**状态:已批准(brainstorming 会话,2026-08-13)**

## 背景

`extensions::project`(第七个扩展化试点,见 `docs/superpowers/specs/2026-08-08-project-info-pane-design.md`)已经落地:项目名、git 分支/脏标、验收次数、目标(标题+标准列表,应用内可编辑)。用户给了一张参考草图(布局:项目名 → 描述占位框 → "文件存储"小节(根目录路径+Git 仓库 remote)→ "项目文档"小节(自动发现的文档链接,可增删)→ "Agent 记忆"小节(自动发现的 agent 记忆链接,可增删)),要求这一轮重构:

1. 撤下"目标"功能的 UI(不删除数据层,以后挪到验收面板)。
2. 新增项目名称编辑、项目描述说明。
3. 展示项目根目录基本信息 + git 信息(含磁盘占用)。
4. 自动搜索可能的文档目录/文件,做成虚拟链接,用户可手动增删。
5. 自动搜索各 agent 的记忆文件,关联为虚拟链接,用户可手动增删。

brainstorming 过程中查证到一处关键事实:`AgentKind` 实际有 7 家(Claude/Codebuddy/
Opencode/Codex/Qoder/Kilo/V8agent),`crates/dozer-app/src/conversation.rs` 已经给
**Claude/Codebuddy/Opencode** 三家定义好了具体的本地存储目录规则(`claude_project_dir`/
`codebuddy_project_dir`/`opencode_project_dir`,统一形状:`~/<agent目录>/projects/
<项目路径把 '/' 换成 '-'>`)——这恰好精确对应草图里画的 Claude/OpenCode/Codebuddy 三个
图标,Agent 记忆发现规则直接复用这三个现成函数,不需要新写路径推导逻辑。

## 目标 / 非目标

**目标**:

1. `extensions::project` 里目标(goal)相关 UI 代码整体删除(`goal_block()`、
   `Message::TitleEditStart`/`TitleEditEvent`/`CriterionAddInputChanged`/
   `CriterionAddSubmit`/`CriterionRemove`、对应 `WorkspaceState` 字段
   `title_editing`/`add_criterion_draft`、这部分单测)。底层 `goal.rs`
   (`Goal`/`load_goal`/`write_goal`/`goal_path`)原样保留不动,供以后验收面板
   复用数据层。`WorkspaceState.goal` 字段本身也删除(面板不再持有目标状态)。
2. 项目名称编辑,daemon 端真改名(不是本地覆盖显示)。
3. 项目描述,本地文件持久化,原生多行输入控件。
4. 项目根目录路径 + git remote URL + 磁盘占用(排除构建产物/依赖目录)展示。
5. 文档虚拟链接:项目首次打开时自动发现一次(根目录直接子项,不递归),写入
   `.dozer/links.json`;之后完全交给用户手动增删,不再自动重新合并。
6. Agent 记忆虚拟链接:同上机制,发现规则复用 `conversation.rs` 的
   `claude_project_dir`/`codebuddy_project_dir`/`opencode_project_dir`。
7. 链接点击:文件类型复用现成的 `Message::PreviewOpenPath` 右侧预览 tab 机制;
   目录类型在面板内就地展开(单层 `read_dir`,不复用 `crates/dozer-app/src/
   project.rs::FileTree`,避免跨模块耦合)。

**非目标**:

- 不做"重新扫描"入口——自动发现只在 `.dozer/links.json` 首次不存在时跑一次。
- 不做磁盘占用的缓存/防抖优化(每次触发点都重新算),性能问题留到实测后再看。
- 不改变 `goal.rs` 数据层的任何行为,`.dozer/goal.md` 格式不变。
- 不新建 `Extension` trait/注册表,不拆独立 crate(沿用现有扩展化模式)。
- 链接路径不做"仓库迁移后自动修复"之类的健壮性处理,存绝对路径,失效链接的
  行为见"错误处理"一节。

## 架构与数据流

### 1. daemon 端:项目改名

`dozer-core::protocol`:

```rust
pub enum Request {
    // ...既有变体...
    /// 项目改名(id 不存在或 name trim 后为空 → Reply::Error)。
    RenameProject { id: i64, name: String },
}
```

成功复用现有 `Reply::Project { project: Option<ProjectInfo> }`(`Some(更新后的项目)`)。

`dozerd::projects::ProjectStore`:

```rust
/// 改名(按 id)。id 不存在 → 返回 Err(不做静默 no-op,调用方需要知道失败)。
pub fn rename(&self, id: i64, name: &str) -> Result<ProjectInfo>
```

实现:`UPDATE projects SET name = ?1 WHERE id = ?2` + 重新 `SELECT` 一次返回
(同 `open()` 的写法)。

`dozer-client::Client`:

```rust
pub async fn rename_project(&self, id: i64, name: String) -> Result<Option<ProjectInfo>>
```

对齐 `open_project` 的请求/应答处理写法。

### 2. `extensions::project::WorkspaceState`

```rust
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    remote_url: Option<String>,
    disk_usage_bytes: Option<u64>,
    project_acceptance_count: Option<u64>,
    description: Option<String>,
    /// 项目名称行内编辑态(None=未在编辑)。
    name_editing: Option<String>,
    /// 描述多行编辑态,`Some` 时面板渲染 `text_editor` 而非静态文本。
    description_editing: Option<iced_widget::text_editor::Content>,
    links: links::LinksState,
    /// 目录类型链接的就地展开集 + 子项缓存(懒加载,`read_dir` 失败记空 Vec)。
    expanded_link_dirs: HashMap<PathBuf, Vec<links::DirRow>>,
    error: Option<String>,
}
```

`goal`/`title_editing`/`add_criterion_draft` 三个字段删除。

### 3. 新增子模块 `extensions/project/links.rs`

数据结构、发现算法、存取、单测都放这里,`extensions/project.rs` 只负责组装
`Message`/`update`/`view`,避免主文件继续膨胀。

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LinkKind { File, Dir }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEntry {
    pub path: PathBuf, // 绝对路径(agent 记忆目录本来就在仓库外,无法相对化)
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LinksState {
    pub docs: Vec<LinkEntry>,
    pub memory: Vec<LinkEntry>,
}

fn links_path(repo: &Path) -> PathBuf { repo.join(".dozer").join("links.json") }

/// 读 `.dozer/links.json`;文件不存在时返回 `None`(调用方据此判断"要不要跑
/// 首次自动发现"),文件存在但损坏时返回 `Some(LinksState::default())`(同
/// `preview_state.rs` 的容错口径,不让损坏文件卡死面板)。
pub fn load(repo: &Path) -> Option<LinksState>

pub fn save(repo: &Path, state: &LinksState) -> std::io::Result<()>

/// 首次发现:根目录直接子项(不递归)。文件名(忽略大小写)以
/// `readme`/`changelog`/`contributing`/`license` 开头,或精确匹配
/// `claude.md`/`agents.md`(指令文件,不算"记忆",归到文档更贴切),或目录名
/// (忽略大小写)精确匹配 `docs`/`doc`/`design`/`documentation`。结果顺序:
/// 文件在前、目录在后,组内按名排序。
pub fn discover_docs(repo: &Path) -> Vec<LinkEntry>

/// 首次发现:`claude_project_dir`/`codebuddy_project_dir`/`opencode_project_dir`
/// 三个目录,存在的才收进结果(不存在的 agent 目录静默跳过,不算错误)。
pub fn discover_memory(repo: &Path) -> Vec<LinkEntry>
```

新增组合入口(比照现有 `load_goal(repo_path)` 的调用方式,由"打开项目"的既有
调用点——`from_restore`/`adopt_project`——在构造 `WorkspaceState::new(..)` 之前
调用,不是 `update()` 里处理):

```rust
/// `links::load` 返回 `None`(文件不存在,首次打开)时跑两个 `discover_*` 拼出
/// 初始 `LinksState` 并立即 `save`;返回 `Some(state)` 直接用,不再跑发现。
pub fn load_or_discover_links(repo: &Path) -> LinksState
```

`WorkspaceState::new` 签名相应从 `new(goal: Option<Goal>)` 改成
`new(description: Option<String>, links: LinksState)`,两个参数分别由调用点
先调 `project_meta::load_description(repo_path)` 和
`links::load_or_discover_links(repo_path)` 拿到。

### 4. `Message`

```rust
pub enum Message {
    // 异步结果(带 project_id,走 with_project,见下方路由说明):
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>, Option<String> /* remote_url */),
    AcceptanceCountLoaded(i64, Option<u64>),
    DiskUsageLoaded(i64, u64),
    NameRenamed(i64, Result<ProjectInfo, String>),

    // 用户交互(走 with_focused_project):
    NameEditStart,
    NameEditEvent(AddrEvent),
    DescriptionEditStart,
    DescriptionEditAction(iced_widget::text_editor::Action),
    DescriptionEditSubmit, // 失焦/显式提交时写盘,见下方"描述保存时机"
    LinkAdd { target: LinkTarget /* Docs | Memory */, path: PathBuf, kind: links::LinkKind },
    LinkRemove { target: LinkTarget, index: usize },
    LinkDirToggle { target: LinkTarget, path: PathBuf },

    // 内核拦截,unreachable!() 于本模块 update()(同 files::Message::OpenFile):
    OpenLink(PathBuf),
}
```

`GitRefreshed` 增加一个字段(`remote_url`),`spawn_project_git_refresh` 同一次
`spawn_blocking` 里多调 `delivery::remote_url(&repo_path)`。

`LinkAdd` 的 `path`/`kind` 由 `main.rs` 侧 `rfd::FileDialog` 选完之后带出来
(见下方"手动加链接"一节),`update()` 本身不碰文件系统对话框。

### 5. 内核路由(`app.rs`)

`Message::Project` 的显式 `project_id` 路由分支(此前只有 `GitRefreshed`/
`AcceptanceCountLoaded`)扩展成:

```rust
Message::Project(
    msg @ (project::Message::GitRefreshed(project_id, ..)
    | project::Message::AcceptanceCountLoaded(project_id, ..)
    | project::Message::DiskUsageLoaded(project_id, ..)
    | project::Message::NameRenamed(project_id, ..)),
) => { /* with_project,原逻辑不变 */ }
```

`NameRenamed(project_id, Ok(updated))` 需要额外把 `Workspace.project`(外层缓存,
顶栏项目页签等读的就是它)同步更新为 `updated`——这是这个面板第一次出现需要
"内核介入"而非纯委托给 `project::update` 的消息,处理方式比照 Files/Acceptance
已有的 kernel-intercept 模式(`handle`/`emit` 或者直接在这个 match 分支里做,
写计划阶段按现有 `files_project_message` 那类辅助函数的形状定)。

`OpenLink(path)` 单独一条分支:

```rust
Message::Project(project::Message::OpenLink(path)) => {
    self.update(Message::PreviewOpenPath(path));
}
```

`project::update()` 内部对 `OpenLink` 分支写 `unreachable!()`(同
`files::Message::OpenFile` 的既有写法)。

`main.rs` 键盘路由:`to_project_title` 改名 `to_project_name`,注释"项目标题
编辑"改"项目名称编辑",指向的消息从 `TitleEditEvent` 改 `NameEditEvent`,链路
位置不变(复用原来空出来的那个优先级槽位,不新增链长)。

### 6. Git 信息 + 磁盘占用

`delivery.rs` 新增:

```rust
/// `git remote get-url origin`;失败或没有 origin 时退化取 `git remote -v`
/// 第一条记录的 URL(没有任何 remote → None)。
pub fn remote_url(repo: &Path) -> Option<String>
```

磁盘占用是独立于 git 刷新的异步任务(避免大仓库的目录遍历拖慢分支/脏标显示),
触发点与 `spawn_project_git_refresh` 相同(项目打开/`adopt_project`/回合结束/
`ProjectFsChanged`),各自独立 `spawn`:

```rust
/// 排除名单是 extensions/project 自己维护的一份新常量,不跟
/// `crates/dozer-app/src/project.rs::HIDDEN`(文件树"要不要显示这一行")复用
/// ——语义不同,这里是"算不算项目真实内容"。
const DISK_USAGE_EXCLUDE: [&str; 7] =
    [".git", "target", "node_modules", "dist", "build", ".venv", "__pycache__"];

/// 递归求和,跳过 DISK_USAGE_EXCLUDE 命中的目录名(整个子树跳过,不下钻)。
/// 读不到的条目(权限/符号链接死链)跳过不计入,不中断整体计算。
pub fn dir_size_excluding(root: &Path, exclude: &[&str]) -> u64
```

`spawn_disk_usage_refresh(project_id, repo_path, io)` 结构比照
`spawn_project_git_refresh`:`tokio::task::spawn_blocking` 里跑
`dir_size_excluding`,完成后 `proxy.send_event(Message::Project(
project::Message::DiskUsageLoaded(project_id, bytes)))`。

### 7. 项目描述

新文件 `crates/dozer-app/src/project_meta.rs`(与 `goal.rs` 平级,不混进同一个
文件——目标和描述是两种独立数据,只是都挂在 `.dozer/` 下):

```rust
fn description_path(repo: &Path) -> PathBuf { repo.join(".dozer").join("description.md") }

/// 文件不存在或内容 trim 后为空 → None。
pub fn load_description(repo: &Path) -> Option<String>

/// 空字符串:仍然写入一个空文件(不是删除文件)——跟 `load_description` 的
/// "trim 后为空 → None"配合,行为等价于清空,不需要额外区分"文件不存在"和
/// "文件存在但是空"两种状态。
pub fn write_description(repo: &Path, text: &str) -> std::io::Result<()>
```

编辑控件用 iced 原生 `text_editor`(多行),**不**走 `main.rs` 的全局自绘拦截链
——现有 `CriterionAdd` 用原生 `text_input` 已验证这条路径在本 App 里能正常收
字符(有 iced 部件持有焦点时,iced 自身的部件事件系统先接住按键,main.rs 那条
拦截只吃"没有部件持有焦点"时落下来的按键)。`DescriptionEditAction` 直接转发
给 `iced_widget::text_editor::Content::perform`;失焦或显式提交
(`DescriptionEditSubmit`)时取 `content.text()`、trim、写盘,同 `goal.rs`
"写失败保留编辑态、允许重试"的既有口径。

### 8. 链接交互

**点击文件类型链接**:发 `Message::Project(project::Message::OpenLink(path))`,
内核拦截转发成 `Message::PreviewOpenPath(path)`,复用现成的右侧预览 tab 机制,
不新写预览逻辑。

**点击目录类型链接**:`LinkDirToggle { target, path }`——已展开则收起(从
`expanded_link_dirs` 移除);未展开则 `std::fs::read_dir(path)` 读一层(目录在
前、按名排序,复用 `icons::icon_for_file` 给文件行选图标),缓存进
`expanded_link_dirs`,面板内缩进渲染。展开的子项本身**不**可再展开(只做一层,
更深的浏览请去 Files 面板或直接点开文件预览)。

**手动加链接**("+"按钮):`main.rs` 侧用 `rfd::FileDialog` 弹出选择框(比照
`Message::ProjectTabPickFolder` 的既有写法,在 `main.rs` 的 `update` 分支里同步
调用 `rfd`,拿到结果后转成 `Message::Project(project::Message::LinkAdd { .. })`)。
rfd 能否同一个对话框里同时选文件或目录待写计划阶段核实;不行的话拆成两个入口
(文件/目录各一个按钮)。选中路径已存在于对应列表(`docs`/`memory`)时视为
no-op(不重复加)。

**删除链接**(每行"×"按钮,复用 `CriterionRemove` 已验证过的"删除+回写失败
原样插回"手法):`LinkRemove { target, index }` 直接从 `docs`/`memory` 对应
`Vec` 移除并 `links::save`,失败则插回原位置并显示 `error`。

### 9. `view`

面板结构(自上而下):项目名(点击进入 `NameEdit*` 编辑态)→ 描述区块(点击进入
`text_editor` 编辑态,无描述时显示占位文案,同草图虚线框观感)→ 分支/脏标行
(原样保留现有 `project_branch_label` 逻辑)→ "N 次验收"副行(原样保留)→
根目录路径 + git remote + 磁盘占用小节 → "项目文档"小节(标题 + "+"按钮,逐行
渲染 `links.docs`,文件/目录两种行样式)→ "Agent 记忆"小节(同上,渲染
`links.memory`)。

目标(goal)相关渲染整段删除。

## 错误处理

- 项目改名:daemon 侧空名 → `Reply::Error`;`NameRenamed(project_id, Err(msg))`
  在面板内显示 `error`,保留编辑态原始输入允许重试(同 `TitleEditEvent` 现有
  口径)。网络/daemon 连接失败视为同一种 `Err` 路径,不单独区分。
- 描述/链接写盘失败 → 面板内 `error` 文案,保留当前编辑态/列表不回滚展示(除
  链接删除失败会显式插回,见上)。
- `.dozer/links.json` 存在但损坏(反序列化失败)→ `links::load` 返回
  `Some(LinksState::default())`,不视为"文件不存在"(不会触发重新自动发现,
  避免损坏文件被静默替换成一份新的自动发现结果、丢失用户之前的手动改动的
  假象——损坏就是空,不是"当作首次打开")。
- 链接目标路径失效(文件被删/改名)→ 点击时 `OpenLink`/`LinkDirToggle` 打开
  失败(`PreviewOpenPath` 打不开 / `read_dir` 报错)按现有预览面板/目录读取的
  既有容错处理,不新增专门的"失效链接"检测或标记,用户手动删除即可。
- 磁盘占用计算中途遇到权限拒绝/符号链接死链的条目 → 跳过不计入,不中断整体
  计算,不报错。
- 非 git 项目 → `remote_url`/`branch`/`worktrees` 均为 `None`/空,面板照常显示
  "—",不算错误状态(同现有 `project_branch_label` 逻辑)。

## 测试策略

- `links.rs`:`discover_docs`(文件名前缀匹配大小写不敏感、`CLAUDE.md`/
  `AGENTS.md` 精确匹配、目录名精确匹配、排序规则、非根目录子项不递归)、
  `discover_memory`(三个 agent 目录部分存在/全不存在)、`load`/`save`
  round-trip、`load` 对损坏文件返回 `Some(default())` 而非 `None`。
- `project_meta.rs`:`load_description`/`write_description` round-trip、空
  内容/纯空白内容视为 `None`。
- `delivery::remote_url`:有 origin、无 origin 有其它 remote、完全没有
  remote、非 git 目录 四种情形。
- `dir_size_excluding`:排除目录不计入、嵌套排除目录整体跳过、空目录返回 0。
- `extensions::project::update`:`NameEditStart` 预填当前名称、`NameRenamed`
  成功/失败两条路径、`LinkAdd`/`LinkRemove`/`LinkDirToggle` 的状态迁移、
  `DiskUsageLoaded`/`GitRefreshed`(含新 `remote_url` 字段)落地赋值。
- `dozerd::projects::ProjectStore::rename`:改名后 `list()`/再次 `open()`
  读到新名字、id 不存在时返回 `Err`。
- 人工验收:改名后顶栏项目页签同步更新;描述输入多行文字、重启 app 后还在;
  首次打开新项目时文档/记忆链接被正确自动发现;删除一条自动发现的链接、
  重新打开面板确认不会复活;点文件链接在右侧预览打开;点目录链接就地展开/
  收起;手动"+"加一条链接;磁盘占用数字在有大 `target/` 目录的仓库(比如
  dozer 自己)里明显小于"不排除"时的数字。

## 依赖变更

无新增依赖(`rfd` 已是既有依赖)。
