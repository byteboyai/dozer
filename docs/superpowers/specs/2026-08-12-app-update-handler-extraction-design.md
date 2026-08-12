# `App::update` 大块内联 arm 抽方法设计

**状态:已批准(brainstorming 会话,2026-08-12)**

## 背景

`App::update`(`app.rs:2332-3847`,约 1516 行)是一个 113 个 match arm 的巨型
函数,对应 `Message` 枚举 74 个变体。逐个测量每个 arm 的行数后发现:混着两类
完全不同的东西——

1. **已经转发给 extension 模块的薄转发**(如 `Message::Todo(msg) =>
   todo::update(...)`),这类没问题,是既定的扩展化模式。
2. **直接写在 match 里的大块内联逻辑**,最大的几个:`AgentStateChanged`
   68 行、`LeftIconSelect` 60 行、`ProjectSlotLoaded` 55 行、
   `ProjectTabOpened` 49 行——总共 **39 个 arm 超过 15 行**,合计约
   1580 行内联逻辑挤在同一个函数体里。

这带来两个具体代价:

- **可读性**:改任何一个 arm 都得在一个 1516 行的函数里定位、且不能不小心
  影响到附近不相关的 arm——这类"改 A 波及 B"的运行时 bug(如
  2026-08-12 修的项目页签拖拽后残留悬停高亮)本身不是这个问题直接导致的,
  但同类"改一处、忘了另一处受影响"的错误在这么大的函数里发生概率更高。
- **审阅成本**:`git diff` 里任何一个 arm 的改动,上下文都是同一个巨型
  函数,审阅者难以只关注"这次到底动了什么逻辑"。

代码库里已经有这个抽取模式的先例——`Message::WebViewMouseUp =>
self.end_tab_drag()`、`Message::TabDragMove{..} =>
self.tab_drag_move(group, index)` 都是"match arm 只剩一行、真正逻辑在一个
具名私有方法里"的既定写法,这次是把同一手法系统化地补齐到其余 39 个大块
arm 上。

## 目标 / 非目标

**目标**:

1. 把 39 个超过 15 行的 match arm 逐个抽成 `impl App` 上的具名私有方法,
   arm 本身收缩成 `Message::X(..) => self.method_name(..),` 一行(或
   `{ self.method_name(..); }`,如果原 arm 是块而非表达式)。
2. 按功能聚类分成 9 组(见下方"架构"一节的完整清单),每组一个可独立
   审阅、独立提交的任务。
3. 方法命名沿用既有先例(`end_tab_drag`/`tab_drag_move`)的动词/名词短语
   风格,不加 `handle_` 前缀。
4. 抽取后的方法体与原 arm 体逐字节一致(只搬家,不改逻辑)——`git diff`
   应该清楚地表现为"删除一段、在别处新增同一段",而不是任何形式的重写。

**非目标**:

- **不新建模块、不新建 extension**。抽取只发生在 `app.rs` 内部,`impl App`
  多几个私有方法,不是这次的"深度重构"选项(brainstorming 已确认选
  "只抽成同文件私有方法")。
- **不改变任何行为**。这是纯粹的代码搬家,不是"顺手改进"——发现某个 arm
  的逻辑看起来可以简化/有 bug,记下来但不在这次改,免得把"消除重复"的
  改动和"修 bug"的改动混在一次 diff 里,审阅时无法区分。
- **不动 15 行以下的 arm**。小 arm 抽成方法反而增加一次跳转,可读性
  净损失。
- **不改 `Message` 枚举、`App` struct 字段、任何 extension 的
  `WorkspaceState`**。纯 `App::update` 内部结构调整。
- **不做与本次抽取无关的顺手清理**(比如发现某个 arm 该合并到隔壁 extension
  模块——那是"更深"的重构,不在这次范围;发现某个变量名起得不好——不顺手
  改,减少 diff 噪音)。

## 架构

### 抽取模式(示例)

以 `Message::AgentStateChanged`(最大的一个,68 行)为例,展示抽取前后:

```rust
// 抽取前(match arm 内联):
Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
    // ...68 行原样逻辑...
}

// 抽取后:
Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
    self.agent_state_changed(project_id, tab_id, agent, state, transcript_path);
}
```

```rust
// impl App 里新增(方法体与原 arm 体逐字节一致,只改了函数签名这一层):
fn agent_state_changed(
    &mut self,
    project_id: ProjectId,
    tab_id: usize,
    agent: AgentKind,
    state: AgentState,
    transcript_path: Option<String>,
) {
    // ...原 68 行逻辑,一字不改地搬过来...
}
```

`AgentStateChanged` 的实际字段类型是 `(ProjectId, usize, AgentKind,
AgentState, Option<String>)`(`app.rs:897` 的 `pub enum Message` 定义——
`tab_id` 是 `usize` 索引不是 `String`,`project_id` 是 `ProjectId` 类型
别名不是裸 `i64`,写这份设计时核对过,不是猜的)。其余 38 个 arm 的具体
字段类型同样以 `pub enum Message`(`app.rs:884` 起)的定义为准,写计划时
逐个从那份定义抄类型,不臆测。若原 arm 体末尾有值(不是 `()`),方法
返回类型跟着定,但目前扫过的 39 个 arm 全部是 `-> ()`(纯副作用,没有
往 `Shell`/`Task` 返回值的情形),预期抽取后也都是 `-> ()`。

### 分组清单(9 组,共 39 个 arm)

| 组 | Arm | 抽取后方法名(建议) |
|---|---|---|
| 项目页签生命周期 | `ProjectSlotLoaded`、`ProjectTabOpened`、`ProjectTabSwitch`、`ProjectTabClose`、`ProjectSelect`、`ProjectFsChanged` | `project_slot_loaded`、`project_tab_opened`、`project_tab_switch`、`project_tab_close`、`project_select`、`project_fs_changed` |
| Database 转发 | `Database(ColumnsLoaded{..})`、`Database(TestConnectionResult(..))`、`Database(TablesLoaded(..))`、`Database(msg)` | `database_columns_loaded`、`database_test_connection_result`、`database_tables_loaded`、`database_message` |
| SSH 转发 | `Ssh(UnknownKeyDetected(..))`、`Ssh(KeyChanged(..))`、`Ssh(TerminalConnectFailed(..))`、`Ssh(TestConnectionResult(..))` | `ssh_unknown_key_detected`、`ssh_key_changed`、`ssh_terminal_connect_failed`、`ssh_test_connection_result` |
| Acceptance | `Acceptance(Open(..))`、`Acceptance(Reject)`、`Acceptance(msg @ (Loaded(project_id,..)｜DiffLoaded(project_id,..)｜Done(project_id,..)))` | `acceptance_open`、`acceptance_reject`、`acceptance_result` |
| Todo | `Todo(DispatchToExisting(..))`、`Todo(DispatchNew(..))`、`Todo(msg)` | `todo_dispatch_to_existing`、`todo_dispatch_new`、`todo_message` |
| Browser | `Browser(msg)`、`Browser(BookmarksMutated(..))`、`Browser(BookmarksLoaded(..))` | `browser_message`、`browser_bookmarks_mutated`、`browser_bookmarks_loaded` |
| 终端 I/O | `TermInput`、`TermOutput`、`TermPaste` | `term_input`、`term_output`、`term_paste` |
| 预览/图标栏/导航 | `LeftIconSelect`、`RightIconSelect`、`PreviewOpenPath`、`PreviewSelectTab`、`SelectTab`、`TopBarHome`、`PaneResized` | `left_icon_select`、`right_icon_select`、`preview_open_path`、`preview_select_tab`、`select_tab`、`top_bar_home`、`pane_resized` |
| 杂项核心 | `AgentStateChanged`、`DeliveryChecked`、`ConversationOpen`、`Search(SearchResults(..))`、`GitLog(LoadMore)`、`Files(msg @ (StatusesRefreshed(project_id,..)｜PasteDone(project_id,..)｜OpDone{project_id,..}))` | `agent_state_changed`、`delivery_checked`、`conversation_open`、`search_results`、`git_log_load_more`、`files_project_message` |

`Acceptance`/`Files` 那两个复合模式都是 `msg @ (A(project_id,..) |
B(project_id,..) | C{project_id,..})` 形式的 OR-pattern——`project_id`
在三个变体里都是首字段,提取时整个 match 模式原样保留在 arm 里,只把
`project_id`/`msg` 作为参数传给新方法,方法体不变。

方法插入位置:优先插在 `impl App` 里已有的同主题方法附近(如项目页签生命
周期几个方法插在 `tab_drag_move`/`end_tab_drag` 旁边);没有同主题precedent
的组,插在 `impl App` 现有私有辅助方法区块的末尾,按分组清单顺序排列,不
打散穿插到无关方法中间。

## 错误处理

不适用——纯代码搬家,原 arm 里已有的错误处理(`?`/`else return`/日志)
原样跟着搬,不新增也不删除任何错误路径。

## 测试策略

这是行为不变的重构,不新增可测逻辑。验证方式:

1. 每组任务完成后,`cargo build -p dozer-app --bin dozer` 编译通过、
   `cargo test -p dozer-app --bin dozer` 产出与开工前完全相同的通过/失败
   数字(484 passed / 2 failed,两个已知的、与本次改动无关的既有失败)。
2. `git diff` 逐组审阅:每组的 diff 应该清楚地表现为"某处删除一段代码、
   别处新增结构相同的一段"(方法签名包一层,方法体不变)——如果 diff
   看起来在改逻辑而不是搬代码,说明抽取过程中手滑改了东西,需要重新核对
   到逐字节一致。
3. 不需要人工截图验证——这次不碰任何渲染/交互代码,`App::view` 与所有
   `extensions/*::view` 均未改动。

## 排期备注

9 组任务之间除了"都改 `app.rs` 的 `impl App` 块"之外没有依赖关系,可以
任意顺序做、分别提交分别审阅。与"tab + icon 按钮共享组件"(另一份已批准
的计划)touch 的是同一个文件的不同部分(那份改的是渲染函数
`project_tab_item`/`left_icon_rail`/`right_icon_rail`,这份改的是
`update` 函数体),两者并行推进冲突面很小,但都改 `app.rs`,建议不要
让两个 agent同时在同一个 worktree 里跑,以免互相踩脚——各开各的分支/
worktree,完成一个合并一个。
