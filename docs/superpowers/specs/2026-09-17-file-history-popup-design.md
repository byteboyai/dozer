# 文件历史对比弹窗设计

**状态:已批准(brainstorming 会话,2026-09-17)**

## 背景

用户想在 Files 面板文件行右键菜单加一项「查看此文件历史」,点击后弹出一个
窗口级弹窗:左侧列出该文件改动过的历史提交,右侧展示"当前磁盘内容 vs 所选
历史提交内容"的 diff,并能一键把文件回滚到某个历史版本。参考图是 JetBrains
风格的 Local History 面板(双栏带行号对照、"外部更改"伪条目、设置齿轮),
但那是 IDE 本地保存历史的概念,我们没有对应机制,只对接 git 提交历史。

现有可复用基础设施:

- Files 面板右键菜单已收拢成 `context_menu_spec()`/`context_menu_items()`
  (`extensions/files/view.rs`),macOS 走原生 NSMenu、非 mac 走 iced 弹层,
  两边共用同一份 `MenuSpec` 数据,加一项很直接。
- Git Log 面板(`extensions/git_log.rs`)已有"选中 commit → 按文件拆
  diff → `colored_diff_lines` 单栏染色渲染"的完整链路(`commit_detail`/
  `DiffFileEntry::patch`),渲染函数 `extensions/diff_render::colored_diff_lines`
  是通用的(泛型消息类型),可以直接复用。但它现在只能"某 commit vs 其父
  commit"整棵树对比,不支持"任意 commit vs 当前工作区某一个文件"这种查询,
  也没有"列出改动过某个文件的所有提交"的能力——这两块都要新写。
- 窗口级弹窗已有先例:`project::project_delete_confirm_popup`/
  `project::scaffold_progress_popup`,状态是 `Option<T>` 直接挂在顶层
  `App`(同 `project_link_menu`/`files::AppState.context_menu` 的既有模式),
  `app/view.rs` 顶层 if-else 链按"哪个弹窗状态非空"选择性地在 `stack!` 里
  叠一层。

## 目标 / 非目标

**目标**:

1. Files 面板文件行(非目录)右键菜单新增「查看此文件历史」,仅当所在项目
   是 git 仓库(`ws_state.git_is_repo`)时出现。
2. 点击后弹出窗口级居中弹窗:左窄栏是该文件的历史提交列表(时间倒序,每行
   commit message 首行 + 时间),右侧是"当前 vs 所选提交"的单栏统一 diff。
3. 弹窗顶部/所选提交行提供「回滚」按钮:点击后把当前磁盘文件内容覆盖成该
   历史版本的字节内容,不自动 `git add`/`commit`,交给用户/agent 自行决定
   要不要提交。回滚成功后弹窗内容(当前内容、diff)原地刷新,同时触发 Files
   树的 dirty 状态刷新。
4. 提交列表、diff 计算均走既有的 `handle.spawn` + `emit` 异步落地模式,不
   阻塞 UI 线程。

**非目标**:

- 不做参考图里的双栏带行号对照排版——复用现有 `colored_diff_lines` 单栏
  染色渲染,视觉上和 Git Log 面板的 diff 区一致。
- 不引入 IDE 本地保存历史("外部更改"伪条目、每次保存都记一条)——版本列表
  只来自 git 提交历史。
- 不做 rename 跟踪(`git log --follow` 语义),只按当前文件路径精确匹配。
- 不改造 Git Log 面板本身,是一个独立的新模块 + 独立弹窗。
- 不新增"从弹窗跳转去 Git Log 面板查看完整提交详情"这类交叉入口。

## 关键语义确认(brainstorming 会话定案)

1. **版本范围**:列出该文件的多个历史提交,可点选切换对比对象(而非只有
   "当前 vs 最近一次提交"单一固定对比)。
2. **diff 样式**:复用现有单栏统一 diff(`colored_diff_lines`),不新建双栏
   行号对照渲染。
3. **回滚**:需要,直接覆盖磁盘文件内容。这是本次唯一一处违反 `CLAUDE.md`
   "预览优先、编辑需要理由"裁决的地方——豁免理由:回滚不是新内容创作,是
   把文件精确恢复成 git 已经记录过的历史字节,一次确定性的机械操作,不涉及
   需要 AI 判断的代码改动,与项目面板"删除项目/修复项目"这类既有的直接操作
   按钮同一性质。

## 架构

### 新模块 `crates/dozer-app/src/extensions/file_history.rs`

自包含单文件模块,风格对齐 `git_log.rs`(数据类型 + `Message` + `State` +
`update()` + view 函数都在一处,规模不大,不预先拆目录)。

**数据类型**:

```rust
pub struct FileHistoryEntry {
    pub oid: git2::Oid,
    pub short_sha: String,
    pub summary: String,       // commit message 首行
    pub author: Option<String>,
    pub time: i64,              // author time,Unix 秒
}

pub struct FileHistorySnapshot {
    pub repo_path: PathBuf,
    pub file_path: PathBuf,     // 仓库相对路径,git 查询用
    pub entries: Vec<FileHistoryEntry>,
}
```

**查询函数**(同步,内部跑在 `handle.spawn` 的阻塞线程池里,不在 UI 线程
调用):

```rust
/// 手写 revwalk:从 HEAD 开始逐提交,对每个提交用
/// `repo.diff_tree_to_tree(parent_tree, tree, Some(&mut opts))` 配合
/// `DiffOptions::pathspec(file_path)` 判断这次提交是否碰过这个文件
/// (`diff.deltas().len() > 0`,pathspec 下推给 git2 做,不用自己在结果里
/// 过滤),命中的收进结果,按 `max_count` 截断。没有 `git log --follow` 的
/// rename 跟踪(见"非目标")。根提交(无父)按空树对比,逻辑同
/// `git_log.rs::commit_detail` 处理根提交的既有写法。注意:文件历史稀疏时
/// (比如仓库有几万个提交、这个文件只被改过 3 次)需要遍历大量提交才能凑够
/// `max_count` 条结果——这是 `git log -- <path>` 的固有特性,不是本实现的
/// bug,不额外做"最多扫描 N 个提交就放弃"的截断(YAGNI,真遇到性能问题再
/// 加)。
pub fn build(repo_path: &Path, file_path: &Path, max_count: usize)
    -> Result<FileHistorySnapshot, String>;

/// `oid` 对应提交的树 vs *当前工作目录*的这一个文件,用
/// `repo.diff_tree_to_workdir(Some(&tree), Some(&mut opts))`(`opts` 同样
/// 配 `pathspec(file_path)`)。`diff_tree_to_workdir` 直接读磁盘上的实时
/// 内容(不是索引/HEAD 里的版本,可能包含未提交改动),不需要自己
/// `std::fs::read` 再手动比较——这也是选它而不是"读两份 blob 手动比较"的
/// 原因(git2 0.21 并未导出 `git_diff_blob_to_buffer` 这个 C API,没有
/// "blob 对内存 buffer"直接比较的安全封装)。两边内容相同时 `diff` 是空
/// (`0` deltas),返回空字符串(`diff_pane_view`/新 view 函数据此显示"内容
/// 相同"提示,同参考图的蓝色提示条语义,但样式沿用本仓已有的"无 diff 内容"
/// 占位文案,不新做提示条组件)。逐行拼 patch 的写法照抄
/// `git_log.rs::commit_detail` 里 `DiffFormat::Patch` 回调那段。
pub fn diff_against_current(repo_path: &Path, file_path: &Path, oid: git2::Oid)
    -> Result<String, String>;

/// 取 `oid` 对应提交树里 `file_path` 的 blob 字节(`tree.get_path(file_path)`
/// → `entry.to_object(&repo)` → `.into_blob()`),写入
/// `repo_path.join(file_path)`。不碰 git 索引,不 `git add`,是纯粹的文件
/// 系统写入。
pub fn rollback_to(repo_path: &Path, file_path: &Path, oid: git2::Oid)
    -> Result<(), String>;
```

**状态与消息**:

```rust
pub struct FileHistoryTarget {
    pub project_id: i64,
    pub repo_path: PathBuf,
    pub file_path: PathBuf,     // 仓库相对路径,git 查询与磁盘读写都用它
                                 // (`repo_path.join(file_path)`)拼出实际路径
}

#[derive(Default)]
pub struct State {
    target: Option<FileHistoryTarget>,
    snapshot: Option<Result<FileHistorySnapshot, String>>,
    selected: Option<git2::Oid>,
    diff: Option<Result<String, String>>,
    rollback_pending: Option<git2::Oid>,
    rollback_error: Option<String>,
}

pub enum Message {
    Close,
    SnapshotLoaded(PathBuf, PathBuf, Result<FileHistorySnapshot, String>), // (repo_path, file_path) 核对仍是当前目标
    SelectCommit(git2::Oid),
    DiffLoaded(git2::Oid, Result<String, String>),
    RollbackRequest(git2::Oid),
    RollbackDone(git2::Oid, Result<(), String>),
}
```

`State` 整体挂在顶层 `App`:`pub(crate) file_history: Option<file_history::State>`,
同 `project_link_menu` 的既有模式——弹窗关闭时整个置 `None`,不保留"上次
看的是哪个文件"这种跨会话状态。

### Files 右键菜单入口

`context_menu_spec()`/`context_menu_items()` 签名加一个 `is_git_repo: bool`
参数(调用方传 `ws_state.git_is_repo`),`!is_dir && is_git_repo` 时插入:

```rust
MenuSpecItem::entry(
    Some(icons::IconKind::History),
    "查看此文件历史",
    files::Message::FileHistoryOpen(target.clone()),
)
```

`files::Message::FileHistoryOpen(PathBuf)`(绝对路径,同其它右键菜单消息
口径)由内核拦截(同 `OpenLink`/`Pick` 的既有例外模式,`files::update()` 收到
会 `unreachable!()`):`App::update` 里换算出仓库相对路径,组出
`FileHistoryTarget`,写入 `self.file_history`,并 `handle.spawn` 异步跑
`file_history::build`,完成后 `emit(Message::FileHistory(file_history::Message::SnapshotLoaded(...)))`。

### 弹窗渲染

新增 `file_history::popup_view(state: &State, window_width: f32) -> Element<Message>`,
布局参照 `project::project_delete_confirm_popup` 的窗口级卡片外壳(CARD 底 +
圆角描边),内部左右分栏:左侧 `Scrollable` 列表(每行提交摘要 + 时间,
选中态用 `card` 背景高亮,同 `links_section` 列表行的既有视觉),右侧
`colored_diff_lines(&patch)` + 顶部一行"当前 vs {short_sha}" + 回滚按钮
(`interactive` 状态在 `rollback_pending.is_some()` 时置 false,避免重复
点击并发回滚)。`app/view.rs` 顶层按 `self.file_history.is_some()` 加一个
新的 `else if` 分支,`stack!` 挂 `scrim` + 本弹窗,同其它窗口级弹窗的既有
接入方式。

### 交互流程

1. 右键文件行 → 选「查看此文件历史」→ 弹窗打开,左栏 loading,异步
   `build()` 落地后填充列表,默认选中列表第一项(最近一次改动)并联动触发
   diff 查询。
2. 点列表某一行 → `SelectCommit(oid)` → 若该 oid 的 diff 尚未缓存,
   `handle.spawn` 跑 `diff_against_current`,落地后 `DiffLoaded`。已经查过
   的 oid 不重复查(`State` 按需加一个 `HashMap<git2::Oid, Result<String,
   String>>` 缓存,避免来回切换重复计算)。
3. 点「回滚」→ `RollbackRequest(oid)` → 置 `rollback_pending = Some(oid)`
   (按钮转不可点)→ 异步 `rollback_to` → `RollbackDone`:成功则清掉
   `rollback_pending`、清空该 oid 的 diff 缓存并重新查一次(应变成"内容
   相同")。回滚本质是一次普通的磁盘文件写入,`workspace::git_watch` 已经
   在监听工作区变更并驱动 `App::project_fs_changed` 自动刷新 git 状态/文件
   树(同 `BranchSwitchDone` 成功后依赖 `git_watch` 自动拾起 HEAD/refs 变化
   的既有口径),不需要另外手动触发一次刷新;失败则置 `rollback_error` 在
   弹窗顶部展示错误条,`rollback_pending` 照样清空。
4. 点关闭/遮罩 → `Message::FileHistory(file_history::Message::Close)` →
   内核拦截把 `self.file_history` 置 `None`。

## 错误处理

- `build()`/`diff_against_current()` 失败(仓库损坏、blob 找不到等)一律
  `Result<_, String>` 落到 `State`,弹窗对应区域展示错误文案,不 panic。
- 弹窗打开期间用户切换了项目 tab 或关闭了当前项目:`SnapshotLoaded`/
  `DiffLoaded`/`RollbackDone` 落地时核对 `project_id`/`repo_path` 是否还
  match `self.file_history` 当前目标,不 match 就丢弃结果(同 `git_log.rs`
  `SnapshotLoaded`/`DetailLoaded` 的既有"是否仍是当前请求"核对写法)。
- 文件从未被 git 记录过(历史列表为空):不隐藏菜单项(省一次右键时的 git
  查询),弹窗展示空列表 + "该文件没有提交记录"提示,右侧 diff 区留空。
- 回滚目标 oid 在该文件历史里已找不到(理论上不会发生,除非仓库在弹窗打开
  期间被外部改写):`rollback_to` 返回错误,同其它失败路径处理。
- 需要在实现阶段核对:弹窗是否会与 Preview 面板的 wry 子视图重叠——按
  `CLAUDE.md` 裁决,原生浮层要盖住 webview 得显式隐藏,不能指望 iced 层级
  自然遮挡。参照 `project_delete_confirm_popup`/`scaffold_progress_popup`
  当前对这个问题的实际处理方式(窗口级弹窗尺寸通常小于整个窗口,是否已经
  绕开了这个问题,还是本来就有缺口),必要时把 `self.file_history.is_some()`
  接入现有 webview 隐藏判断链路。

## 测试策略

- 纯逻辑单测(不需要真实 git 仓库的部分):
  - `FileHistoryEntry`/`FileHistorySnapshot` 的字段组装。
  - `State` 的 `SnapshotLoaded`/`SelectCommit`/`DiffLoaded`/
    `RollbackRequest`/`RollbackDone` 各分支状态迁移(含"结果与当前目标不
    match 时丢弃"的核对逻辑)。
- 对真实临时 git 仓库(`tempfile` + `git2::Repository::init`,同
  `git_log.rs`/`delivery.rs` 现有测试的既有写法)的集成测试:
  - `build()`:一个文件跨 3 个提交被改动 2 次、1 次无关提交不改这个文件,
    验证结果只含相关的 2 条,顺序正确。
  - `diff_against_current()`:当前磁盘内容与某历史版本相同 → 空字符串;
    不同 → patch 非空且含预期的 `+`/`-` 行。
  - `rollback_to()`:执行后磁盘文件字节与历史 blob 一致。
- 手动验证(`cargo run -p dozer-app`):右键文件 → 查看历史 → 列表/diff
  正确 → 回滚 → 磁盘内容变化且 Files 树 dirty 图标更新 → 弹窗 diff 变
  "内容相同" → 非 git 仓库项目/从未提交过的文件两种边界场景。

## 排期备注

- 独立 worktree 分支开发,完工经代码审阅通过后再合并 main(本仓库既有
  硬性要求,见"写 plan 要求独立分支"的历史教训)。
- 建议任务拆分:①`file_history.rs` 数据层(`build`/`diff_against_current`/
  `rollback_to` + 单测)②`State`/`Message`/`update()` 消息驱动③弹窗 view +
  Files 右键菜单接入 + `app/view.rs` 顶层挂载④人工 GUI 验收(逐条对照上面
  "测试策略"手动验证清单)。①②可并行,③依赖①②,④收尾。
