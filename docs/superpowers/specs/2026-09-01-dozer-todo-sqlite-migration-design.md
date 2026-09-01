# Todo 存储从文件迁移到 SQLite

**状态:已批准(brainstorming 会话,2026-09-01)**

## 背景

Todo 面板(`crates/dozer-app/src/extensions/todo.rs`,3612 行)现状是两份
独立文件,完全在 `dozer-app` 本地,`dozerd` 全程不参与:

- **正文**:`<repo>/.dozer/todo.md`,标准两态 checkbox(`- [ ]`/`- [x]`),
  git 可追踪。这是刻意的设计选择——`2026-08-06-dozer-todo-panel-design.md`
  当时的裁决是"跟着仓库走",明确不引入 daemon 侧协议或 SQLite 表,理由是
  markdown 让 agent 可以直接用文件编辑工具读写任务,不需要额外集成。
- **元数据 sidecar**:`dozer_core::paths::config_dir()/todo_meta.json`,存
  每条任务的派发记录(`DispatchRecord{session_id, dispatched_at}`)、计划
  日期(`plan_date`,"MM-DD")、完成时间(`completed_at`)。GUI 本地,不进
  git。任务没有稳定 id,靠 `todo_line_key(text)`(trim 后文本的
  `DefaultHasher` 哈希)跟正文关联——代价是"改了任务文字就跟丢这条的全部
  本地元数据",v1 时是已知且接受的权衡。

`dozerd` 侧已经用同一个 `dozer.db`(rusqlite,`Mutex<Connection>`)托管了
`ProjectStore`/`AcceptanceStore`/`BookmarkStore`/`TranscriptStore`/
`SessionSummaryStore` 五套结构化数据,`dozer-mcp` 是面向外部 CLI agent 的
只读 MCP server(现状两个工具:`get_preview_context`/
`submit_session_summary`,不涉及 todo)。

用户下一步想让 Todo 支持"把整个列表指派给一个 agent,agent 自主轮询列表、
自动完成任务、对任务写评论"。这需要:(1)任务有稳定 id(评论/指派要挂在
具体某条任务上,哈希碰撞和"改文字丢关联"不可接受);(2)轮询/自动完成
发生在一个不依赖 GUI 是否打开的常驻进程里。这两点决定了原来"跟着仓库走"
的裁决需要推翻:brainstorming 会话内确认改为 **SQLite 作为唯一权威源,
挂在 dozerd 侧的 `dozer.db`**,`.dozer/todo.md` 完全退役。

这份 spec 是**第一个子项目**:只做"把现有功能原样搬到 SQLite 上"这一层
地基,不实现指派/轮询/自动完成/评论本身——那是后续独立 spec。

## 目标 / 非目标

**目标**:

1. 新增 `crates/dozerd/src/todo.rs::TodoStore`,复用 `dozer.db`,建表存放
   任务正文+全部元数据(见下方数据模型),仿照 `BookmarkStore` 的
   `CREATE TABLE IF NOT EXISTS` + `Mutex<Connection>` 模式。
2. `dozer-core::protocol` 新增 `Todo*` 系列 `Request`/`Reply`,`dozer-client
   ::Client` 加对应方法,风格对齐现有 `AddBookmark`/`RemoveBookmark`/
   `ListBookmarks` 三件套。
3. `dozer-app` 的 `extensions/todo.rs` 里所有直接 `std::fs` 读写(正文和
   `todo_meta.json`)改成走 `Client` 发协议消息;`WorkspaceState` 里纯 UI
   态(草稿、焦点、拖拽进行态、日历视图、flash 计时、过滤/搜索)不动。
4. **移除 MARKDOWN 视图 tab**(`TodoViewMode`/`markdown_editing`/
   `markdown_draft`/`ViewModeSet`/`MarkdownEditStart`/`MarkdownEvent` 及其
   UI 一并删除)——它是 `.dozer/todo.md` 全文可编辑视图,退役后没有对应的
   落盘目标,brainstorming 会话内确认直接去掉,不做只读降级。
5. `OpenProject` 处理时,`dozerd` 对每个项目做**一次性**历史导入:读
   `cwd/.dozer/todo.md` 解析出任务,按现状哈希规则去 `todo_meta.json` 捞
   对应元数据一并写入新表;导入后打标记,之后不再重复导入(即使用户后来
   清空了任务列表、或 markdown 文件仍留着旧内容)。
6. `dozer-mcp` 新增 4 个工具:`list_todos`/`add_todo`/`toggle_todo`/
   `edit_todo_text`——对齐外部 CLI agent(Claude Code/opencode 等)原来
   靠直接编辑 `.dozer/todo.md` 能做到的事(查看/新增/勾选/改文字),
   markdown 退役后这是它们唯一的操作入口,缺了会导致外部 agent 当场失能。
7. 原文件(`.dozer/todo.md`、`todo_meta.json`)迁移后**不自动删除**,留在
   磁盘原样不动,只是 Dozer 自身不再读写——删除意味着往用户仓库里造一次
   自动 git diff,不在这次范围内。
8. 迁移完成后 `cargo build`(全 workspace)、`cargo test -p dozerd -p
   dozer-app -p dozer-core -p dozer-client -p dozer-mcp`、`cargo clippy
   --all-targets`、`cargo fmt` 全绿;真实 GUI 里验证现有全部交互(增/改/
   勾选/拖拽排序/过滤搜索/指派到已有会话/计划日期)行为与迁移前一致。

**非目标**:

- **不做**整个列表批量指派给 agent、agent 自主轮询、自动完成任务、任务
  评论——这是下一阶段独立 spec,本次只交付存储地基。
- **不新增单条任务删除**。现状 footbar"清空列表"按钮是占位 no-op(`update`
  里直接空分支,UI 已就位但逻辑未接),迁移后保持 no-op,不趁机实现。
- **不改任何现有 UI 交互/视觉**,除了上面明确列出的 MARKDOWN tab 移除。
- **不做推送式实时更新**。现状 GUI 靠 1 秒自限速轮询(`TODO_POLL_INTERVAL`,
  `main.rs:275`)检测外部变化,迁移后轮询动作从"看文件 mtime"换成"发
  `ListTodos`",触发节奏不变。推送式(复用协议里已有的 `Reply::AgentEvent`
  推送模式)是未来可能的优化,不在本次范围。
- **不处理多进程并发写冲突超出现状**。SQLite 单连接 + `Mutex` 天然把并发
  写请求串行化,比现状"文件行文本匹配失败就放弃这次写入"更健壮,不需要
  额外设计冲突处理。

## 架构与数据流

### 数据模型

```sql
CREATE TABLE IF NOT EXISTS todos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL,
    text TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    rank INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    plan_date TEXT,
    dispatch_session_id TEXT,
    dispatch_at_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_todos_project_order
    ON todos(project_id, done, rank);

CREATE TABLE IF NOT EXISTS todo_legacy_imported (
    project_id INTEGER PRIMARY KEY,
    imported_ms INTEGER NOT NULL
);
```

原来两份存储(`TodoItem{text,done}` 来自 markdown 行 + 按
`project_id+hash(text)` 存的 `TodoTaskMeta{dispatch,plan_date,
completed_at}`)合并成一张表、一行一条任务,`id` 是新增的稳定身份,取代
`todo_line_key` 哈希。

**排序**:`rank` 是单个 per-project 整数序列,覆盖待办+已完成全部任务。
展示查询固定 `ORDER BY done ASC, rank ASC`——待办按 rank 升序排在前,
已完成按 rank 升序排在后,天然复现现状"已完成行保持原位置、整体沉底
展示"的语义,不需要额外分支。新增任务"置顶"= 新 rank 取小于当前所有
待办最小 rank 的值(表为空则从 0 开始)。拖拽重排只在待办子集内发生,
把被拖任务的 rank 改到目标相邻两个待办 rank 之间;`todo_legacy_imported`
记录该 project 是否已经跑过一次性历史导入,`OpenProject` 据此判断要不要
再检查磁盘上的 markdown 文件。

`TodoInfo` 协议结构体(字段对应上表,供 `Reply` 使用):

```rust
pub struct TodoInfo {
    pub id: i64,
    pub project_id: i64,
    pub text: String,
    pub done: bool,
    pub rank: i64,
    pub created_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub plan_date: Option<String>,
    pub dispatch_session_id: Option<String>,
    pub dispatch_at_ms: Option<u64>,
}
```

三态展示(`TodoState::{Pending,InProgress,Done}`)继续是纯函数在
`dozer-app` 侧推导,不进数据库:`done` 为真直接 Done;否则看
`dispatch_session_id` 是否有值且对应 session 是否还存活(GUI 本地已知的
`ws.tabs` 状态)→ InProgress;否则 Pending。这块逻辑(`todo_display_state`)
不变。

### 历史数据导入

`OpenProject` 处理时,`dozerd`:

1. 查 `todo_legacy_imported` 有没有这个 `project_id` 的记录,有则跳过
   (整个导入流程 no-op)。
2. 没有 → 读 `cwd/.dozer/todo.md`(不存在则视为空列表),用现状
   `parse_todo` 同款规则解析出 `Vec<TodoItem>`,按文件出现顺序赋
   `rank`(待办和已完成都参与同一个递增序列,保留原始相对顺序,由
   `ORDER BY done, rank` 在展示时天然分组)。
3. 对每条解析出的任务,用现状 `todo_line_key(text)` 算哈希,去
   `dozer_core::paths::config_dir()/todo_meta.json`(`dozerd` 直接用共享
   路径帮助函数读,同 `BackfillProjectTranscripts` 已有的"dozerd 读取
   项目目录/配置目录"先例)捞对应的 `dispatch`/`plan_date`/
   `completed_at`,一并写入 `todos` 表。
4. 写入 `todo_legacy_imported(project_id, now_ms)`。
5. 全程失败(文件读取错误、解析异常)不中断 `OpenProject` 本身——记
   `tracing::warn!` 后按"这个项目没有历史 todo"处理,并仍然写入迁移标记
   (避免每次 `OpenProject` 都重新尝试同一个读不了的文件)。

原文件在导入后不删除、不改动。

### 协议扩展

`dozer-core::protocol` 新增:

```rust
// Request
ListTodos { project_id: i64 },
AddTodo { project_id: i64, text: String },
ToggleTodo { id: i64, done: bool },
EditTodoText { id: i64, text: String },
ReorderTodo { id: i64, after_id: Option<i64> }, // 挪到 after_id 之后;None = 待办块最前
SetTodoPlanDate { id: i64, plan_date: Option<String> },
RecordTodoDispatch { id: i64, session_id: String },

// Reply
Todos { todos: Vec<TodoInfo> },
Todo { todo: TodoInfo },
```

`ToggleTodo`/`EditTodoText`/`ReorderTodo`/`SetTodoPlanDate`/
`RecordTodoDispatch` 统一返回 `Reply::Todo`(更新后的整行),`id` 不存在
一律 `Reply::Error`(对齐现状 `RenameProject`/`RemoveProject` 的错误处理
风格)。`ToggleTodo` 在 `done` 从假变真时顺带把 `completed_at_ms` 设为
当前时间,从真变假时清空——这段"翻转联动"逻辑现状在 `AppState::
set_completed_at` 里,搬进 `TodoStore::toggle` 内部一次性做完,不再需要
`dozer-app` 侧单独调用一次元数据写入(现状 `set_done` 是"改文件 + 若文本
未变则调 `app_state.set_completed_at`"两步,搬迁后合并成一次 UDS 调用)。

`dozer-client::Client` 加 `list_todos`/`add_todo`/`toggle_todo`/
`edit_todo_text`/`reorder_todo`/`set_todo_plan_date`/`record_todo_dispatch`
七个方法,内部走既有的 UDS 请求/响应模式(参照 `add_bookmark`/
`remove_bookmark`/`list_bookmarks`)。

### `dozer-app` 面板改造

`extensions/todo.rs` 里纯 I/O 部分整体替换:

- 删除:`parse_todo`/`replace_todo_line`/`prepend_todo_item`/
  `move_pending_to`/`todo_path`/`reload_from_disk`/`todo_line_key`/
  `TodoMetaState`/`meta_load`/`meta_save`/`load_from`/`save_to`/
  `file_path`/`AppState`(整个结构体连同 `meta`/`record_dispatch`/
  `set_plan_date`/`set_completed_at`/`meta_for` 一起,职责搬进
  `TodoStore`)。
- 删除:`TodoViewMode`/`markdown_editing`/`markdown_draft`/
  `ViewModeSet`/`MarkdownEditStart`/`MarkdownEvent`/`cancel_markdown_edit`
  及对应 UI(MARKDOWN tab 整体移除)。
- 新增:一个薄的异步调用层,`Message` 里增删改类消息(`Toggle`/
  `AddSubmit`/`ContentEdit` 提交/`DragEnd`/`CalendarPick`/
  `DispatchToExisting`)改成调 `Client` 对应方法并等待 `Reply`,成功后
  用返回的 `TodoInfo`/`Vec<TodoInfo>` 刷新 `WorkspaceState::items`(类型
  从 `Vec<TodoItem>` 换成 `Vec<TodoInfo>`,`TodoItem` 类型退役)。
- `WorkspaceState` 保留字段:`add_draft`/`add_focused`/`add_input_height`/
  `filter`/`selected_row`/`flash`/`scroll_to_top`/`search`/
  `search_draft`/`search_focused`/`dispatch_open`/`dispatch_anchor`/
  `calendar_open`/`calendar_view`/`calendar_anchor`/`editing_content`/
  `content_edit_focused`/`content_edit_focus_pending`/`drag`——这些是纯
  UI 交互态,与底层存储无关,不改。
- 轮询:`App::poll_todo_if_visible`(`app.rs:2679`)自限速逻辑
  (`TODO_POLL_INTERVAL` = 1 秒,由更高频的唤醒事件顺带驱动,不是独立
  定时器)不变,轮询动作从"`stat` 文件 mtime,变了才重读"改成"每次到点
  直接发 `ListTodos`,用返回结果整体替换 `items`"(`TodoInfo` 派生
  `PartialEq`,列表未变时替换成本可忽略,不需要额外的"是否变化"判断)。

### `dozer-mcp` 新增工具

`crates/dozer-mcp/src/server.rs` 新增 4 个 `#[tool]` 方法,复用现有
`session_id → sessions 列表 → project_id` 解析模式(`fetch_context` 同款
逻辑,新增 `resolve_project_id` 辅助函数给这 4 个工具共用):

- `list_todos`:无参数,返回当前会话所属项目的任务列表(`id`/`text`/
  `done`,不暴露 `dispatch`/`plan_date` 等纯 GUI 侧字段——外部 agent
  原本通过文件编辑也看不到这些,MCP 只还原它们原有的能力面)。
- `add_todo { text: String }`:新增一条任务(置顶,同 GUI 行为)。
- `toggle_todo { id: i64, done: bool }`:勾选/取消勾选。
- `edit_todo_text { id: i64, text: String }`:改任务文字。

不暴露 `reorder`/`plan_date`/`dispatch`——这些原本就是纯 GUI 侧交互,
外部 agent 通过编辑 `.dozer/todo.md` 从来碰不到(markdown 文件里没有
这些信息),不需要在 MCP 补出对应能力。

## 错误处理

- `TodoStore` 的写方法(`toggle`/`edit_text`/`reorder`/`set_plan_date`/
  `record_dispatch`)对不存在的 `id` 返回 `Err`,`server.rs` 映射成
  `Reply::Error`——对齐现状 `RenameProject`/`RemoveProject` 对不存在
  `id` 的处理方式,不是新引入的模式。
- 历史导入失败(见上"历史数据导入"第 5 步)不阻塞 `OpenProject` 成功
  返回,只是这个项目看起来"没有历史任务",符合现状"文件不存在/读失败
  按空列表处理,不 panic"的既有哲学。
- `dozer-mcp` 新增的 4 个工具,`Client` 调用失败(dozerd 未启动/id 不
  存在)一律 `McpError::internal_error`,同现有 `submit_session_summary`
  的错误映射方式。
- GUI 侧 UDS 调用失败(dozerd 连接断开等):现状 `commit_add_task`/
  `set_done` 等函数里文件写失败是 `tracing::warn!` 后放弃这次操作、不
  崩溃;搬迁后 `Client` 调用失败保持同等降级——记日志,`WorkspaceState`
  维持调用前的展示状态,不做重试/弹窗(与其余 `dozer-app` 内 UDS 调用
  失败的既有处理风格一致)。

## 测试策略

1. `TodoStore` 单元测试(仿 `bookmarks.rs` 现有测试结构):增/删除/切换
   完成/改文字/重排/计划日期/派发记录的基本 CRUD;`ORDER BY done, rank`
   查询顺序断言(待办在前按 rank 升序、已完成沉底按 rank 升序)。
2. 历史导入测试,覆盖四种场景:无 `.dozer/todo.md` 文件、有正文无
   `todo_meta.json`、正文+元数据都有且能通过哈希正确关联、同一
   `project_id` 二次 `OpenProject` 不重复导入(即使这期间往
   `todo.md` 追加了新内容,第二次也不读)。
3. `ToggleTodo` 完成态联动测试:`done` 假→真写入 `completed_at_ms`,
   真→假清空,与现状 `completed_at_for_toggle` 纯函数行为等价。
4. `dozer-mcp` 4 个新工具的集成测试(仿现有 `tests/*.rs` 对
   `get_preview_context`/`submit_session_summary` 的测试方式):
   session→project 解析失败时的错误路径、正常路径下 JSON 形状断言。
5. `dozer-app` 侧现状已有的纯函数测试(`filter_todos`/
   `todo_display_state`/`completed_at_for_toggle`/`today_ymd` 等日历
   相关函数)不受影响,保持通过。
6. `cargo build`(全 workspace)、`cargo test -p dozerd -p dozer-app -p
   dozer-core -p dozer-client -p dozer-mcp`、`cargo clippy --all-targets
   -- -D warnings`、`cargo fmt -- --check`。
7. 真实 GUI 验证(单测覆盖不到端到端体验):新增/勾选/拖拽排序/过滤
   搜索/指派到已有会话/计划日期全部行为与迁移前一致;确认 MARKDOWN
   tab 已从 UI 消失;找一个有历史 `.dozer/todo.md` 的项目打开,确认
   历史任务和派发/计划日期正确导入;重启 `dozerd` 后任务不丢;用一个
   接了 dozer-mcp 的外部 agent(如 opencode)验证 4 个新工具可用。

## 排期备注

搬迁量级中等偏大,风险集中在两处:(1)`extensions/todo.rs` 现有 3612
行里 UI 交互态和文件 I/O 混在同一批函数里(如 `set_done`/
`commit_content_edit` 都是"读文件→改文本→写文件→触发 UI 副作用"一条
龙),拆分时要小心不要把 UI 副作用(flash 高亮、滚动定位、选中态)漏改;
(2)`rank` 字段的重排算法(`ReorderTodo` 在两个相邻 rank 之间插入,连续
高频拖拽可能耗尽整数间隙)现状 `move_pending_to` 是对文件整体重写、没有
这个问题,SQLite 版需要在实现阶段确认"必要时批量重新编号"的兜底策略。
建议实现阶段顺序:先落 `TodoStore` 骨架 + 历史导入(可独立用单测验证,
不接触 GUI/协议),再落协议扩展 + `dozer-client` 方法,再落 `dozer-app`
面板改造(建议先把 I/O 全部换成 UDS 调用、行为验证通过后再删 MARKDOWN
tab,两步分开减少同时改动面),最后落 `dozer-mcp` 4 个新工具。
