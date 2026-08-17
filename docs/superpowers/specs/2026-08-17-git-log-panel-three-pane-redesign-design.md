# Git Log 面板三栏重构设计

**状态:草稿(brainstorming 会话,2026-08-17,待用户审阅)**

## 背景

`crates/dozer-app/src/extensions/git_log.rs`(2026-08-07 已完成"扩展化试点"改造,
自己的 `Message`/`State`/`update`/`view`,内核 `app.rs` 只包一层
`Message::GitLog(..)` 转发,细节见
`docs/superpowers/specs/2026-08-07-git-log-extension-pilot-design.md`)现状是
"提交拓扑图 + 固定宽度详情栏"的单栏(内部两块)布局:

- 左边是 `GitLogCanvas`,`gleisbau` 布局引擎算出的分支拓扑图,commit 是圆点,
  父子关系用直线连接,点圆点选中一个 commit。
- 选中 commit 后,右边弹出一块固定宽度(`DETAIL_WIDTH = 460px`)的详情栏
  (`detail_view`):该 commit 改动的每个文件按顺序纵向平铺,文件名下面紧跟着
  这个文件完整的 unified diff 文本(单色,不分行染色)。
- 没有"点文件单独看这个文件 diff"的两级交互——所有文件的 diff 一次性摊开在
  同一个可滚动列里。
- 面板顶部另有一条 worktree 速览条(点击可切到其它 worktree 当新项目页签)。

用户给了一张手绘线框图(参考截图),要把面板重构成三个独立区域:左侧 Git 面板
(commit 列表 + 分支切换)、右上文件列表面板、右下"具体的修改内容"(选中文件的
diff)面板。这是本次 spec 的输入。

## 目标 / 非目标

**目标:**
1. 面板从"拓扑图 + 固定宽详情栏"改成三栏可拖拽布局:左(commit 列表)|
   右上(文件列表)/右下(diff 内容)——即一条竖向分割线 + 右侧一条横向分割线,
   两条都可拖拽调整比例,布局态随项目持久化(镜像现有 `PanelDims` 机制)。
2. 左侧 commit 列表从 Canvas 自绘的分支拓扑图简化成普通可滚动列表:每行
   圆点/图标 + `short_sha` + 时间戳 + `summary`,普通提交用 `git-commit-vertical`
   图标,合并提交(≥2 parent)用 `git-merge` 图标区分,分支/tag 标签跟在后面。
   **有损简化**:merge commit 具体从哪条分支合并、拓扑连线不再可视化,只保留
   时间线性顺序 + 图标标出"这是个合并"。
3. 新增"选中文件"这层状态:点文件列表某一行,右下面板展示该文件在选中 commit
   里的具体 diff,逐行按 `+`/`-`/上下文染色(复用 `acceptance.rs::diff_view`
   的染色规则),不再像现状那样把所有文件的 diff 一次性摊平展示。
4. 左侧面板底部新增分支切换下拉,点击真正执行 `git checkout`(复用
   `delivery::local_branches`/`delivery::checkout_branch` 两个数据函数,UI 视觉
   风格照抄 `files.rs::branch_picker_popup`,但状态/消息是 `git_log` 模块自己
   的一套——两个扩展模块之间不共享状态,只能通过各自的 `Message` 与内核交互,
   这是"Git Log 扩展化试点"定下的既有原则,这次重构继续遵守)。
5. worktree 速览条保留,挪到左侧 Git 面板顶部(标题下方、commit 列表上方),
   逻辑不变。

**非目标:**
- **不改数据获取层**:`commit_detail()`(一次性返回文件列表 + 每个文件 patch)
  已经够用,不新增/不拆分 git 数据函数。
- **不引入" Extension trait / 注册表"**——继续 2026-08-07 定下的阶段 1(静态
  分发,模块自己的 Message+State+update+view,内核包装转发)路线,不升级到
  阶段 2。
- **不改"Git Log 状态 `App` 级共享、不按项目分"的现状行为**——这是一个独立的
  产品决策,不跟这次布局重构混着做(与 2026-08-07 spec 的既有口径一致)。
- **不做 merge commit 的分支拓扑可视化补偿**(比如给 merge 画一条简化连线、
  或在 hover 时弹出"这是从 X 分支合并"这类信息)——`git-merge` 图标是唯一的
  合并标记,拓扑细节的取舍已经跟用户确认过,这次不做任何补偿性可视化。
- **不做根提交(0 parent)的图标区分**——已跟用户确认,只需要区分"普通/合并"
  两种,根提交按"普通"处理,用 `git-commit-vertical`。
- **不改动分支切换的语义**——是真正的 `git checkout`(会改写工作区文件),
  跟 `files.rs` 现有分支切换行为一致,不引入"仅切换查看视角、不动工作区"这
  第二套机制。
- **不新增撤销/储藏(stash)/合并冲突处理等 git 操作**——分支切换沿用
  `checkout_branch` 现有的"dirty 时锁定切换"保护(见"关键语义确认"),不新增
  处理 dirty 的其它路径(比如自动 stash)。

## 关键语义确认(brainstorming 会话定案)

- **分支下拉 = 真正 `git checkout`**,不是"仅切换查看历史的视角"。跟
  `files.rs` 现有分支切换语义完全一致,复用同一套 dirty 检测:当前分支有
  未提交改动时,除当前分支外的其余分支全部禁用(参考
  `files.rs::branch_picker_popup` 的 `lock_others` 逻辑),但 git_log 模块
  自己独立实现这套判断——**不直接调用 `files::branch_picker_popup` 或读
  `files::WorkspaceState` 的字段**,只共享底层数据函数
  `delivery::local_branches`/`delivery::checkout_branch`/`delivery::dir_status`。
- **commit 列表从拓扑图简化成线性列表**,放弃分支拓扑可视化。合并提交靠
  `git-commit-vertical`(普通)/ `git-merge`(合并,≥2 parent)两种图标区分,
  不做第三种(根提交)。
- **左右分割线(commit 列表 | 右侧区)与右侧横向分割线(文件列表 | diff
  内容)都可拖拽**,布局随项目持久化,镜像 `PanelDims`/`Divider`/
  `apply_column_drag`/`split_portions` 现有机制。**右侧横向分割线是代码库
  里第一条纵向(上下)拖拽分割线**——现有 `Divider` 枚举的六个变体全部是
  左右分割,没有任何纵向拖拽先例,这次要新写拖拽方向/光标/几何计算,不能
  照抄现有横向分支直接套用。
- **worktree 速览条保留**,位置从"标题+header 之后、commit 图之前"挪到左侧
  Git 面板顶部(标题下方,commit 列表上方),逻辑/交互(`ProjectTabOpen`)
  不变。

## 架构与数据流

### 1. `git_log::State` 新增字段

```rust
pub struct State {
    // ——— 现有字段,不变 ———
    cache: Option<GitLogSnapshot>,
    error: Option<String>,
    selected: Option<git2::Oid>,
    detail: Option<Result<CommitDetail, String>>,
    pending: Option<(PathBuf, usize)>,
    restore_after_load: Option<git2::Oid>,

    // ——— 新增:选中文件 ———
    /// 右上文件列表当前选中的文件路径(`CommitDetail.files[].path`)。切
    /// commit 时(`SelectCommit`/`DetailLoaded` 落地)一并重置:新 commit
    /// 的 `detail` 到手后默认预选第一个改动文件,避免右下角空白。
    selected_file: Option<String>,

    // ——— 新增:分支切换(独立于 files.rs,不共享状态)———
    /// 面板底部分支下拉是否展开。
    branch_picker_open: bool,
    /// 当前仓库的本地分支列表(`delivery::local_branches` 的结果缓存)。
    branches: Vec<String>,
    /// 当前分支名(`None` = detached HEAD,信息来自 `GitLogSnapshot.head_branch`,
    /// 不重复存一份——`branches`/切换用得到分支名列表,当前分支直接读
    /// `cache.as_ref().and_then(|c| c.head_branch())`,这里不新增冗余字段)。
    /// 分支切换请求进行中的标记(禁用下拉、显示"切换中…")。
    branch_switch_pending: bool,
}
```

### 2. `git_log::Message` 新增变体

```rust
pub enum Message {
    // ——— 现有变体,不变 ———
    SelectCommit(git2::Oid),
    LoadMore,
    ProjectTabOpen(PathBuf),
    DetailLoaded(PathBuf, git2::Oid, Result<CommitDetail, String>),
    SnapshotLoaded(PathBuf, usize, Result<GitLogSnapshot, String>),

    // ——— 新增:选中文件 ———
    SelectFile(String),

    // ——— 新增:分支切换 ———
    BranchPickerOpen,
    BranchPickerClose,
    /// 打开下拉时若还没缓存过分支列表,内核负责异步查一次
    /// (`delivery::local_branches`,同步阻塞函数,内核 `spawn_blocking`),
    /// 落地即这条消息。
    BranchesLoaded(PathBuf, Vec<String>),
    /// 点某个分支——内核截获处理(同 `LoadMore`/`ProjectTabOpen` 的既有
    /// 例外模式),不会转发到 `update`。
    BranchSwitch(String),
    BranchSwitchDone(Result<(), String>),
}
```

`SelectCommit`/`DetailLoaded` 落地逻辑追加:成功拿到新 `detail` 时,
`selected_file` 重置为 `detail.files.first().map(|f| f.path.clone())`(有文件
则预选第一个,无改动文件则 `None`)。

### 3. 分支切换的内核截获模式

跟 `LoadMore`(需要仓库路径)、`ProjectTabOpen`(需要转成
`Message::ProjectTabOpen` 切项目页签)同一个既有模式:`BranchPickerOpen`
首次展开且 `state.branches` 为空时,内核在 `Message::GitLog` 分发里单独一支
处理——查仓库路径、`spawn_blocking` 跑 `delivery::local_branches`、落地发
`BranchesLoaded`。`BranchSwitch(name)` 同理,内核单独处理:查仓库路径、
`spawn_blocking` 跑 `delivery::checkout_branch`、落地发 `BranchSwitchDone`。
`update()` 对这两种消息的分支保留 `unreachable!`(镜像 `LoadMore`/
`ProjectTabOpen` 现有写法),不是遗漏。

`BranchSwitchDone(Ok(()))` 落地后:清 `branch_picker_open`/
`branch_switch_pending`,并触发一次 `request_refresh`(checkout 后 HEAD/
提交历史变了,commit 列表要重新拉;镜像 `files.rs::BranchSwitchDone` 成功后
"再刷一次分支信息"的既有模式,这里刷的是 commit 列表而不是分支信息)。
`request_refresh` 的 `max_count` 用 `state.cache_max_count()`(现有方法,
读当前缓存的窗口大小,无缓存则回落 `DEFAULT_MAX_COMMITS`)——保留用户
"加载更多"过的窗口大小,不因为切了个分支就把列表重置回默认 200 条。
`Err(e)` 落地:清 `branch_switch_pending`,把 `e` 存进 `state.error`(复用
现有错误展示路径,不新增分支切换专属的错误 UI)。

### 4. 左右分割线(commit 列表 | 右侧区)

复用现有横向拖拽机制,新增:
- `PanelDims` 加一个字段 `git_log_split: f32`,`default_panel_dims()`/
  `sanitize_panel_dims()` 按其余四个 split 同款处理(`default_split_ratio()`
  初始值、`clamp_split` 夹范围)。
- `Divider` 加一个变体 `GitLogSplit`。
- `apply_column_drag` 加一支,逻辑与 `TodoSplit`/`ProjectSplit` 完全一致
  (`pair_content_width(left_zone_width(..))` 算比例、`clamp` 到
  `min_split_ratio()`/`max_split_ratio()`)。
- `git_log::view()` 内部用 `split_portions(git_log_split)` 算出的
  `(list_portion, content_portion)` 拼 `row![commit_list.width(FillPortion(list_portion)), divider_bar(Divider::GitLogSplit), right_side.width(FillPortion(content_portion))]`,
  跟 Todo/Project 面板现有拼法一致。

### 5. 右侧横向分割线(文件列表 | diff 内容)——新机制,无先例

这是代码库里第一条纵向(上下)可拖拽分割线,现有 `Divider`/
`apply_column_drag`/`Message::ColumnDrag{logical_x}` 全部是为左右分割设计的,
不能直接照搬,需要新写一套平行机制:

- `PanelDims` 再加一个字段 `git_log_file_diff_split: f32`(命名与
  `git_log_split` 区分"左右"与"上下"两条线),同款默认值/夹范围处理。
- 新枚举或者扩展现有 `Divider`(写计划阶段定,倾向新增一个独立枚举
  `RowDivider` 只装 `GitLogFileDiffSplit` 一个变体,与 `Divider` 语义上
  区分"这是纵向的",而不是往 `Divider` 塞一个语义不同的横向枚举里易读性
  更差——但如果写计划时发现共用 `Divider` + 一个 `is_vertical` bool 更省
  代码量,允许调整,不是这个 spec 要锁死的实现细节)。
- 新消息 `Message::RowDrag { divider: RowDivider, logical_y: f32 }` /
  `Message::RowDragEnd`,`main.rs` 的 `CursorMoved` 处理里新增一段:当前有
  纵向拖拽在进行时,按窗口高度换算 `logical_y`(镜像现有 `logical_x` 换算,
  `(cursor_phys.y / scale) as f32`),发 `RowDrag`。
- 新纯函数 `apply_row_drag(state, divider, window_height, logical_y) -> PanelDims`,
  数学镜像 `apply_column_drag`:算出"右侧区域的可用高度"(`git_log` 面板
  内容区高度,减去标题/worktree 条/commit 列表 header 等固定高度部分——
  具体常量写计划时从 `view()` 实际布局量出),比例 = `(logical_y - 区域顶部
  y 偏移) / 可用高度`,`clamp` 到 `min_split_ratio()`/`max_split_ratio()`。
- `git_log::view()` 右侧区用 `column![file_list.height(FillPortion(top)), horizontal_divider_bar(RowDivider::GitLogFileDiffSplit), diff_pane.height(FillPortion(bottom))]`。
- `divider_bar`(现有横向分割线渲染,鼠标样式 `ResizingHorizontally`)需要一个
  纵向姊妹函数(鼠标样式 `ResizingVertically`,几何上是一条水平线而不是竖线),
  函数名/具体参数留给写计划定,建议镜像现有 `divider_bar` 签名。

### 6. commit 列表:从 Canvas 拓扑图改成 iced 列表

- `GitLogCanvas`(`canvas::Program` 实现,`567-670` 行)整个删除,不再需要
  Canvas/自绘几何(`ROW_HEIGHT`/`COL_WIDTH`/`DOT_RADIUS`/`row_center`/
  `track_color`/`TRACK_COLORS` 等布局常量与函数一并删除或视情况保留——
  时间戳格式化可能用得上现有的时间处理惯例,写计划时核对)。
- `CommitRow` 删除 `column`/`color_idx`/`parents` 三个字段(拓扑图专属,
  列表不需要位置/连线信息),新增 `time: i64`(`git2::Commit::time()` 的
  author time,Unix 秒数)、`is_merge: bool`(`commit.parent_ids().count() >= 2`,
  `build()` 里从 `commit.parents` 的长度直接推导,不需要额外 git2 调用)。
- `build()` 精简:不再需要 `gleisbau` 的 `column`/`color_idx`/分支布局分析
  ——**这里有个决策点**:`gleisbau` 这个依赖除了拓扑图布局外,还提供了
  `refs`(分支/tag 标签)和 `head_branch` 推导的现成能力,如果只是要一份
  按时间倒序的线性 commit 列表 + refs 标签,理论上可以换成纯 `git2` revwalk
  实现,去掉 `gleisbau` 依赖;但 `refs` 标签的等价 git2 实现(`Repository::
  references()` 遍历 + 归属判断)有一定工作量,且 `gleisbau` 的 revwalk 本身
  没有性能问题(现有测试显示 200 commit 窗口正常运行)。**倾向保留
  `gleisbau::graph::Builder` 继续跑,只是从结果里只取 `oid`/`summary`/
  `refs`/`parents.len()`/时间这几个字段,不取 `column`/`color_idx`——省去
  重写 revwalk+refs 归属逻辑的成本,牺牲一点"用不上的布局计算"性能开销
  (200 个 commit 量级下不构成问题)**。写计划阶段如果验证下来
  `gleisbau::graph::Builder::build()` 强制要求跑完布局分析、拿不到"只要
  revwalk 不要布局"的轻量入口,再重新评估要不要自己手写 revwalk。
- 新增列表渲染函数(替代 `GitLogCanvas`),`scrollable(column![...])`,每行
  `row![icon(commit_icon(is_merge)), text(short_sha), text(format_commit_time(time)), text(summary), refs_pills(refs)]`,
  整行包 `MouseArea`(或 `button`)`on_press(Message::SelectCommit(oid))`,
  选中态左侧金色竖条高亮(对齐 Todo/Files 现有选中行视觉语言,替代原来
  Canvas 里"选中提交描边加粗圆圈"的高亮方式)。
- `commit_icon(is_merge: bool) -> IconKind`:`false` → `IconKind::
  GitCommitVertical`(新增 `IconKind` 变体),`true` → `IconKind::GitMerge`
  (新增 `IconKind` 变体)。两个图标资源需要从 Lucide 下载
  (`git-commit-vertical.svg`/`git-merge.svg`,已核实 Lucide 图标库真实存在
  这两个 slug)放进 `crates/dozer-app/assets/icons/`,在 `icons.rs` 的
  `IconKind` 枚举与 `include_bytes!` 分支里注册,跟现有 `git-branch`/
  `git-graph` 同样的接线方式(`icons.rs:70/127/179/202` 是现有先例)。
- `format_commit_time(unix_secs: i64) -> String`:格式对齐设计稿
  `2026-08-17 10:22:31`(本地时区,`YYYY-MM-DD HH:MM:SS`)。项目里已有类似
  时间格式化的地方(如 Todo 面板完成时间戳、对话列表 `modified_ms`),写
  计划时核对现有惯例用的时间/日期库(`chrono`?标准库?)与格式化风格,
  尽量复用同一套,不新增格式约定。

### 7. 文件列表面板(新增)

- 渲染 `state.detail`(`Ok(detail)` 时)里的 `detail.files`,每行
  `row![text(status_glyph(status)).color(status_color), text(path)]`,
  样式/配色规则原样照抄现有 `detail_view` 里的 `status_glyph`/颜色映射
  (`Added`→GREEN、`Deleted`→RED、其余→CYAN)。
- 点击一行发 `Message::SelectFile(path)`,选中态视觉同 commit 列表(左侧
  金色竖条)。
- 空状态(`detail.files.is_empty()`)/加载中(`state.detail.is_none()`)/
  出错(`Err`)三态渲染逻辑照抄现有 `detail_view` 对应分支。

### 8. diff 内容面板(重写渲染,复用染色逻辑)

- 根据 `state.selected_file` 从 `state.detail` 的 `Ok(detail).files` 里找到
  对应 `DiffFileEntry`,取其 `patch` 字符串。
- 逐行染色规则直接照抄 `acceptance.rs::diff_view`(`491-529` 行):
  `+` 开头 GREEN、`-` 开头 RED、其余 DIM,等宽字体
  (`Font::MONOSPACE`)、`caption_sm()` 字号、`TERM_BG` 背景——这次重构统一
  两处的 diff 渲染观感,不再是 git_log 自己一套单色文本、acceptance 面板
  另一套染色文本。
- `selected_file` 为 `None`(比如该 commit 没有改动文件)时显示"无文件改动"
  占位文案(照抄现有 `detail_view` 对应分支)。
- `truncated` 标记的提示文案(`"… diff 过长,已截断显示"`)保留,渲染逻辑
  不变,只是外层从"单色纯文本"换成"逐行染色文本"。

### 9. worktree 速览条位置调整

`worktree_strip()` 函数本身不改,只改 `view()` 里的调用顺序:从"`header`
之后、commit 图之前"挪到"标题(`home_panel_head`)之后、commit 列表之前",
即左侧 Git 面板内部顺序变成
`标题 → worktree_strip → commit_list(scrollable) → 分支切换下拉`。

### 10. 分支切换下拉(左侧面板底部)

自绘下拉,视觉风格照抄 `files.rs::branch_picker_popup`(`CARD` 底/
`BORDER` 描边、当前分支 GOLD 高亮+指示点、dirty 时锁定其余分支并加
`(Uncommitted)` 后缀),但**不是 window-wide overlay**——`files.rs` 那版是
在 `App::view` 顶层 `stack!` 里用 `Padding` 手动定位钉在 git 底栏上方,这次
改成 `git_log::view()` 内部自包含的局部 `stack!`(仅覆盖左侧 Git 面板范围,
不需要 `App::view` 层面的 orchestration),下层垫透明 `MouseArea` 承接
"点别处收起",符合本模块"自己的 Message+State+update+view,不需要内核
额外接线"的既定原则。当前分支名从 `state.cache.as_ref().and_then(|c| c.
head_branch())` 取,不新增字段。

## 组件边界

- **`git_log` 模块**:拥有 commit 列表、文件列表、diff 内容、分支切换下拉、
  两条分割线的全部状态/消息/渲染。不知道自己被 `app.rs` 包在哪个外层
  `Message` 类型里,不读 `files::WorkspaceState`/`Workspace` 的任何字段
  (跟 2026-08-07 试点定下的边界一致)。
- **`app.rs`(内核)**:负责"当前聚焦项目的仓库路径"这类跨面板知识(继续
  截获 `LoadMore`/`ProjectTabOpen`/`BranchPickerOpen`(首次)/`BranchSwitch`
  四种需要仓库路径的消息)、`PanelDims`/两条新 split 字段的持久化、纵向
  拖拽的 `main.rs` 鼠标事件转发。
- **`delivery.rs`**:`local_branches`/`checkout_branch`/`dir_status` 三个
  现成函数直接复用,不改签名,不新增。
- **`icons.rs`**:新增两个 `IconKind` 变体 + 两份 vendored SVG,机械性接线,
  不改现有变体。

## 错误处理

沿用现有两条既有降级逻辑,新增一条:
- `commit_detail`/`build` 失败:红字展示(不变)。
- `DetailLoaded`/`SnapshotLoaded` 落地时核对是否仍对得上当前状态,不对就
  静默丢弃(不变)。
- **新增**:`BranchesLoaded`/`BranchSwitchDone` 同样需要"结果是否还对得上
  当前仓库路径"的核对(项目切换后旧请求落地不该污染新项目的分支列表/
  切换态)——镜像 `DetailLoaded` 的核对模式。`BranchSwitchDone(Err(e))`
  展示到 `state.error`(复用现有错误展示位置,不新增专属 UI 区域)。

## 测试策略

- **纯函数/数据层**:
  - `build()` 新增字段(`time`/`is_merge`)的正确性——`is_merge` 对已知
    merge commit(现有测试 `build_against_real_repo` 已确认 Dozer 仓库里
    至少有一个 merge)应为 `true`,对普通提交应为 `false`;`time` 应该是
    合理的 Unix 秒数(非零、不早于仓库创建时间的粗略下限)。
  - `format_commit_time()`:给定已知 Unix 秒数,输出格式符合
    `YYYY-MM-DD HH:MM:SS`(具体时区处理写计划时定,写一个固定输入固定
    输出的用例)。
- **状态机**(`update()` 新增分支):
  - `SelectFile`:落地后 `selected_file` 被设置。
  - `SelectCommit`/`DetailLoaded` 成功落地后 `selected_file` 被重置为
    `detail.files.first()` 的路径(有文件时)/`None`(无文件时)。
  - `BranchesLoaded`:仓库路径匹配时落地 `branches`,不匹配时丢弃(镜像
    `DetailLoaded` 的核对测试模式,含匹配/不匹配两个用例)。
  - `BranchSwitchDone(Ok(()))`:清 `branch_picker_open`/
    `branch_switch_pending`,产生"需要 request_refresh"的信号(具体返回值
    形状——是否复用现有 `Option<Message>` 返回通道——写计划时定,建议
    复用 `SnapshotLoaded` 那种"返回 `Some(next)` 让内核递归分发"的既有
    模式,不新增第二套"模块请求内核做事"的通道)。
  - `BranchSwitchDone(Err(e))`:清 `branch_switch_pending`,`error` 被设置。
- **拖拽几何**(纯函数,不依赖真实鼠标事件):
  - `apply_column_drag` 新增的 `GitLogSplit` 分支:输入 `logical_x`/
    `window_width`,输出比例落在 `[min_split_ratio(), max_split_ratio()]`
    区间,镜像现有 `TodoSplit` 等测试用例写法。
  - 新增的 `apply_row_drag`:同样验证输出比例落在合法区间内,含边界值
    (`logical_y` 超出面板范围两端时应被 `clamp` 而不是产生非法比例)。
  - `sanitize_panel_dims` 新增的两个字段同样被夹进合法范围(镜像现有
    四个 split 字段的既有测试模式)。
- **编译期防回归**:`cargo build -p dozer-app && cargo test -p dozer-app &&
  cargo clippy -p dozer-app --all-targets && cargo fmt --check` 全绿。
- **人工验收**(`cargo run -p dozer-app`):
  1. 打开一个有 merge commit 历史的项目,进 Git Log 面板,确认 commit
     列表正常倒序展示、merge commit 图标跟普通提交视觉区分明显。
  2. 点一个 commit → 右上文件列表刷新;点某个文件 → 右下 diff 内容刷新,
     染色正确(增删行颜色对)。
  3. 拖拽左右分割线、右侧上下分割线,比例正常响应,重开 app 后比例保持
     (持久化)。
  4. 点分支下拉,切一个不同的分支:确认真的 `git checkout` 了(工作区
     文件变化)、commit 列表刷新成新分支的历史、当前有未提交改动时其余
     分支正确被禁用。
  5. worktree 速览条位置/点击切换行为跟改造前一致。
  6. 切项目/切 tab 后面板内容跟着对,不残留上一个项目的状态(`App` 级
     共享缓存这条现状行为不能被破坏,同 2026-08-07 spec 的验收口径)。

## 依赖变更

- 无新增 crate 依赖(`gleisbau`/`git2` 继续用,倾向保留 `gleisbau` 只取部分
  字段,见"架构与数据流"第 6 节的决策点说明)。
- 新增两个 vendored 图标资源文件:`crates/dozer-app/assets/icons/
  git-commit-vertical.svg`、`crates/dozer-app/assets/icons/git-merge.svg`
  (从 https://lucide.dev 下载,与现有 `git-branch.svg`/`git-graph.svg`
  同来源同许可证,已确认这两个 slug 在 Lucide 图标库真实存在)。

## 待写计划时确认的实现细节(不阻塞 spec 批准,但要在计划里给出具体方案)

1. `RowDivider` 是新增独立枚举还是复用 `Divider` 加方向标记——本 spec 倾向
   前者,允许写计划时按实际代码量调整。
2. `apply_row_drag` 算比例时,"右侧区域可用高度"具体怎么从 `view()` 的实际
   布局量出(标题栏/worktree 条/面板内边距占多少固定高度)——需要写计划
   时读实际渲染代码给出准确的 `theme::geometry` 常量或计算式。
3. `gleisbau::graph::Builder` 能否只跑 revwalk+refs 不跑布局分析(性能/
   API 可行性核实),核实不通过则保留全量布局计算只是不消费 `column`/
   `color_idx` 字段。
4. `format_commit_time` 具体用什么时间库/复用哪个现有格式化函数。
5. `BranchSwitchDone` 触发 `request_refresh` 的具体消息返回形状(是否复用
   `Option<Message>` 递归分发通道)。
</content>
