# Todo 任务分类树

**状态:已批准(brainstorming 会话,2026-09-01)**

## 背景

[[2026-09-01-dozer-todo-sqlite-migration-design.md]] 把 Todo 存储从
`.dozer/todo.md` 迁到 `dozerd` 侧 `dozer.db` 的 `todos` 表,任务有了稳定
`id`,但列表本身仍是单个项目下的一份扁平列表,按 `done`/`rank` 排序,没有
任何分组/归类能力。项目任务一多(个位数到几十条),用户想要能自己建
树状分类结构把任务分门别类,类似文件系统的文件夹。

`dozer-app` 已经有一个成熟的树形 UI 先例可以照抄:`extensions/files.rs`
的文件树(`project.rs::FileTree`,懒加载子目录、`expanded: HashSet`
记展开态、`visible_rows()` 把树按当前展开态拍平成一份 `Vec<TreeRow>`
喂给 view)。调研确认这套模式没有被抽成 `byteui` 共享组件,是手写在
`files.rs` 里、跟 `PathBuf` 强绑定的;本次分类树数据量小(通常几个到
几十个节点),不需要照搬"懒加载"这部分,但"扁平存储 + 展开态 + 拍平成
行"这个整体思路值得复用。

## 目标 / 非目标

**目标**:

1. 新增 `crates/dozerd/src/todo_category.rs::CategoryStore`,复用
   `dozer.db`,建 `todo_categories` 表(见数据模型),仿照 `TodoStore`/
   `BookmarkStore` 的 `CREATE TABLE IF NOT EXISTS` + `Mutex<Connection>`
   模式,与 `TodoStore` 平级、独立文件、独立结构体。
2. `todos` 表新增可空列 `category_id`,`TodoInfo` 协议结构体同步加
   `category_id: Option<i64>` 字段。`NULL` = 未分类,不强制任务必须挂
   分类。
3. `dozer-core::protocol` 新增 `Category*` 系列 `Request`/`Reply` 和
   `SetTodoCategory`,`dozer-client::Client` 加对应七个方法(见协议
   扩展一节)。
4. `dozer-app` 的 `extensions/todo.rs` 面板改造成左侧分类树 + 右侧过滤
   后的平铺任务列表两栏布局:
   - 左侧树:固定的"全部"/"未分类"两个伪节点钉在最顶,下面是用户自建的
     真实分类节点,支持展开/收起、新建子分类/同级分类、重命名、删除
     (级联规则见下)、上移/下移(同级内交换 `rank`)、移动到...(reparent,
     校验不能挂到自己的子孙下面成环)。
   - 右侧列表:选中"全部"显示该项目全部任务;选中"未分类"只显示
     `category_id IS NULL` 的任务;选中某个真实分类节点,显示挂在该
     节点**及其全部子孙节点**下的任务(汇总展示,不要求逐层下钻才能看到
     深层任务)。过滤/汇总全部是客户端内存计算(GUI 已经一次性拿到该
     项目全部 `TodoInfo` + 全部 `CategoryInfo`),不新增服务端查询参数。
     现有排序(`done`/`rank`)、拖拽重排、过滤搜索、日历/派发等交互在
     过滤后的子集上原样工作,不改动这些既有逻辑本身。
   - 每条任务行新增一个分类标签 chip(显示当前分类名或"未分类"),点击
     弹出一个轻量选择器(复用同一套树形行渲染)供选择/摘除分类,调
     `SetTodoCategory`。
5. **不做树节点拖拽**(建分类/移动分类/挂任务分类全部走右键菜单和点击
   选择器,原因见下方"决策记录")。
6. 迁移完成后 `cargo build`(全 workspace)、`cargo test -p dozerd -p
   dozer-app -p dozer-core -p dozer-client`、`cargo clippy --all-targets
   -- -D warnings`、`cargo fmt -- --check` 全绿;真实 GUI 里验证:建
   多级分类、任务挂分类/摘分类、选中父节点看到子孙任务汇总、删除有
   子孙/任务的分类后任务正确降级未分类、reparent 成环被拒绝、重启
   `dozerd` 后分类树不丢。

**非目标**:

- **不做跨项目共享分类**。分类树按 `project_id` 隔离,与现状 `todos`
  表的隔离方式一致(brainstorming 已确认:单项目内)。
- **不做多分类标签**。一条任务同一时刻只能挂一个分类(brainstorming
  已确认:单一分类,`category_id` 是单值不是关联表)。
- **不做树节点/任务的拖拽移动**。全部走右键菜单动作(新建子分类/新建
  同级分类/重命名/上移/下移/移动到.../删除)和任务行的分类选择器点击。
  调研确认 `dozer-app` 目前没有任何内部节点拖拽重排/重挂载的机制可以
  复用(`files.rs` 只有"从 Finder 拖文件进来"这种外部拖拽),要做的话
  是从零建一套光标命中测试 + drop-target 高亮 + drop 消息的机制,超出
  本次范围。按 YAGNI 先用菜单交出这个能力,真用起来觉得菜单慢再单独
  立项做拖拽。
- **不给 `dozer-mcp` 开放分类相关工具**。`list_todos` 现有返回结构里
  顺带带上新增的 `category_id` 字段方便外部 agent 读取当前归属,但
  `add_category`/`set_todo_category` 这类写工具本期不加——分类是用户
  自己组织任务的手段,brainstorming 已确认本期仅 GUI,MCP 先不动,先把
  数据模型和 GUI 交互做稳,后续再评估是否要开放给 agent 自动归类。
- **不改任务本身的既有字段/交互**(`text`/`done`/`rank`/`plan_date`/
  `dispatch_session_id` 等),分类是新增的正交维度,不影响现状排序/
  展示/派发逻辑。

## 决策记录(brainstorming 会话结论,供实现阶段参照)

- 作用域:单项目内,不跨项目共享。
- 任务归属:单一分类(不是多标签)。
- UI 交互模式:左侧树形面板 + 右侧过滤后平铺列表(不是单栏嵌套树)。
- 父节点聚合:选中有子分类的父节点时,右侧汇总显示其全部子孙分类下的
  任务(不是只显示直接挂载的任务)。
- 删除级联:删除分类节点时,整棵子树上原本挂靠的任务全部降级为未分类
  (`category_id = NULL`),不级联删除任务本身。
- MCP 暴露:本期仅 GUI 操作,不给 agent 开放分类相关写工具。
- 节点移动:不做拖拽,全部走右键菜单/点击选择器。

## 架构与数据流

### 数据模型

```sql
CREATE TABLE IF NOT EXISTS todo_categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL,
    parent_id INTEGER,           -- NULL = 顶层节点
    name TEXT NOT NULL,
    rank INTEGER NOT NULL,       -- 同一 parent_id 下的兄弟排序
    created_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_todo_categories_project_parent
    ON todo_categories(project_id, parent_id, rank);
```

`todos` 表新增一列(需要一条 `ALTER TABLE todos ADD COLUMN category_id
INTEGER` 迁移语句,在 `TodoStore::new` 建表逻辑里追加,仿照
`transcripts.rs::open_on_pre_existing_db_without_is_error_column_adds_it`
现有的"运行时探测列是否存在、不存在则 `ALTER TABLE` 补上"先例,保证老
`dozer.db` 升级后不丢已有任务数据):

```sql
ALTER TABLE todos ADD COLUMN category_id INTEGER;  -- NULL = 未分类
```

不使用 SQLite 外键约束做级联——删除分类子树时任务要降级为未分类而不是
被删除,这是业务语义,不是简单的 `ON DELETE CASCADE`/`ON DELETE SET
NULL` 能直接表达清楚的(尤其是"递归收集子孙分类"这一步),级联逻辑整个
在 Rust 层手动实现。

`CategoryStore`(`crates/dozerd/src/todo_category.rs`)方法:

- `list(project_id) -> Vec<CategoryInfo>`:该项目全部分类节点,扁平返回,
  树形结构由调用方按 `parent_id` 自己拼(数据量小,不做懒加载,一次性
  全量拿,对齐 `ListProjects`/`ListTodos` 现状"全量返回"的风格)。
- `add(project_id, parent_id: Option<i64>, name: &str) -> CategoryInfo`:
  新分类追加到目标 `parent_id` 下兄弟节点末尾(`rank` = 当前同级最大
  `rank` + 1,该 `parent_id` 下还没有节点则从 0 开始)。
- `rename(id, name: &str) -> CategoryInfo`
- `delete(id) -> Result<()>`:递归收集 `id` 自身及全部子孙分类 id(Rust
  层用 `parent_id` 关系逐层查,树深度不会大,不需要 SQL 递归 CTE),
  一次事务内 `UPDATE todos SET category_id = NULL WHERE category_id IN
  (...)` 后删除这些分类行。`id` 不存在返回 `Err`。
- `reparent(id, new_parent_id: Option<i64>) -> Result<CategoryInfo>`:
  校验 `new_parent_id` 不等于 `id` 本身、也不是 `id` 的任何子孙(沿
  `parent_id` 链向上走到根,若途中遇到 `id` 则拒绝,返回 `Err`),校验
  通过后追加到新父节点子级末尾(同 `add` 的 `rank` 计算方式)。
- `move_sibling(id, direction: CategoryMoveDirection) -> Result<CategoryInfo>`:
  取同一 `parent_id` 下按 `rank` 排序的相邻兄弟,与之交换 `rank`;已经
  在最前/最后时对应方向为 no-op(返回当前状态,不报错)。

`TodoStore` 新增:

- `set_category(id, category_id: Option<i64>) -> Result<TodoInfo>`:
  更新任务的 `category_id` 并返回整行,`id` 不存在返回 `Err`(与现状
  `toggle`/`edit_text` 同样的错误处理风格)。传入的 `category_id` 是否
  真实存在,由 `server.rs` handler 调 `CategoryStore` 校验后再落库(见
  错误处理一节),`TodoStore` 本身不知道 `CategoryStore` 的存在,保持两个
  store 互不依赖。

`CategoryInfo` 协议结构体:

```rust
pub struct CategoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub rank: i64,
    pub created_ms: u64,
}
```

`TodoInfo` 新增字段:

```rust
pub category_id: Option<i64>,
```

### 协议扩展

`dozer-core::protocol` 新增:

```rust
// Request
ListCategories { project_id: i64 },
AddCategory { project_id: i64, parent_id: Option<i64>, name: String },
RenameCategory { id: i64, name: String },
DeleteCategory { id: i64 },
ReparentCategory { id: i64, new_parent_id: Option<i64> },
MoveCategorySibling { id: i64, direction: CategoryMoveDirection },
SetTodoCategory { id: i64, category_id: Option<i64> },

// 新增枚举
pub enum CategoryMoveDirection { Up, Down }

// Reply
Categories { categories: Vec<CategoryInfo> },
Category { category: CategoryInfo },
```

`SetTodoCategory` 复用现有 `Reply::Todo`(更新后的整条任务)。
`DeleteCategory` 复用现有 `Reply::Ok`。全部失败路径复用现有
`Reply::Error { message }`。

`dozer-client::Client` 加 `list_categories`/`add_category`/
`rename_category`/`delete_category`/`reparent_category`/
`move_category_sibling`/`set_todo_category` 七个方法,内部走既有
`roundtrip` 模式,对齐 `list_todos`/`add_todo` 那批方法的写法。

`ListTodos` 本身不变,不加 `category_id` 查询参数或过滤能力——按分类
过滤和子孙汇总全部是 `dozer-app` 侧对已经拿到的全量 `Vec<TodoInfo>` +
`Vec<CategoryInfo>` 做内存计算。

### `dozer-app` 面板改造

`extensions/todo.rs` 的 `WorkspaceState` 新增:

- `categories: Vec<CategoryInfo>`:随 `ListTodos` 同一轮轮询/项目打开
  时一并拉取刷新(新增一次 `list_categories` 调用,与现有
  `TODO_POLL_INTERVAL` 自限速轮询同节奏)。
- `category_expanded: HashSet<i64>`:展开的分类 id,纯 UI 态,不落盘
  (对齐 Files 面板 `FileTree::expanded` 同样"只在内存里、不持久化"的
  处理)。
- `category_selected: CategoryFilter`(新枚举 `All | Uncategorized |
  Node(i64)`,默认 `All`):当前选中的过滤节点。
- `category_context_menu: Option<CategoryContextMenu { x, y, id }>`:
  右键菜单弹出态,对齐现有 `ProjectLinkContextMenu`/`PreviewTabMenu`
  同款结构。
- `category_picker: Option<CategoryPickerState { x, y, todo_id }>`:
  任务行分类标签 chip 点击后弹出的轻量选择器状态。

纯函数(单测覆盖,不碰 GUI 状态):

- `visible_category_rows(categories: &[CategoryInfo], expanded:
  &HashSet<i64>) -> Vec<CategoryTreeRow>`:按 `parent_id` 拼树、按
  `expanded` 展开态拍平成行(`id, name, depth, has_children, expanded`),
  对齐 `files.rs::visible_rows()` 的模式。
- `category_descendants(categories: &[CategoryInfo], root: i64) ->
  HashSet<i64>`:算子孙节点 id 集合,供"选中父节点看子孙任务汇总"和
  "reparent 目标是否合法"两处复用。
- `filter_todos_by_category(todos: &[TodoInfo], categories:
  &[CategoryInfo], filter: CategoryFilter) -> Vec<&TodoInfo>`:
  `All` 原样返回、`Uncategorized` 过滤 `category_id.is_none()`、
  `Node(id)` 过滤 `category_id` 命中 `{id} ∪ category_descendants(id)`。

视图:左侧树用 `icons::icon_button_entry` 做展开箭头(复用已有组件,
对齐 CLAUDE.md 关键裁决"新增 icon 按钮优先复用统一组件"),右键菜单
复用现有 `menu::shell`/`item` 壳(同 `ProjectLinkContextMenu` 的接线
方式);右侧任务列表沿用现有渲染逻辑,只是输入换成
`filter_todos_by_category` 算出的子集,拖拽重排/勾选/编辑/日历/派发
等交互不改动。

任务行新增分类标签 chip:显示当前 `category_id` 对应的分类名(查不到
或为 `None` 显示"未分类"),点击打开 `category_picker`(复用
`visible_category_rows` 同一份树形行渲染,选中后调
`SetTodoCategory`,选"未分类"传 `category_id: None`)。

## 错误处理

- `CategoryStore::delete`/`reparent`/`move_sibling` 对不存在的 `id`
  返回 `Err`,`server.rs` 映射成 `Reply::Error`,对齐现状
  `RenameProject`/`TodoStore::toggle` 的处理方式。
- `reparent` 检测到成环,返回带明确文案的 `Err`(如"不能移动到自己的
  子分类下"),`server.rs` 透传给 `Reply::Error`,GUI 侧按现状 UDS 调用
  失败的降级方式处理(`tracing::warn!` + 维持调用前的展示状态,不弹窗/
  不重试)。
- `SetTodoCategory` 的 `category_id` 若不为 `None`,`server.rs` handler
  先调 `CategoryStore::list` 确认该 id 存在且属于同一 `project_id`
  (跨项目挂分类没有意义,防止 GUI bug 传错 id 把任务挂到别的项目分类
  下),不存在/跨项目一律 `Reply::Error`,再调
  `TodoStore::set_category`。
- `DeleteCategory` 找不到 `id` 时返回 `Err`,不是静默 no-op(对齐
  `ProjectStore::rename`/`remove` 现状风格)。

## 测试策略

1. `CategoryStore` 单元测试(仿 `TodoStore` 现有测试结构):新增/重命名/
   删除(含级联降未分类)/按 project 隔离/reparent 成功/reparent 成环
   拒绝(直接子节点、跨多层子孙两种情况都要覆盖)/上移下移交换
   rank/首尾边界 no-op。
2. `TodoStore::set_category` 单测:正常挂分类、摘分类(传 `None`)、
   不存在 id 报错。
3. `dozer-client` 集成测试(`against_real_daemon.rs` 追加用例):七个
   新方法的 roundtrip,仿现有 `list_todos`/`add_todo` 测试写法。
4. `dozer-app` 纯函数单测:`visible_category_rows`(多层嵌套、展开态
   变化)、`category_descendants`(叶子节点/中间节点/整棵树根节点三种
   情况)、`filter_todos_by_category`(三种 `CategoryFilter` 分支 + 父
   节点汇总子孙任务的场景)。
5. `cargo build`(全 workspace)、`cargo test -p dozerd -p dozer-app -p
   dozer-core -p dozer-client`、`cargo clippy --all-targets -- -D
   warnings`、`cargo fmt -- --check`。
6. 真实 GUI 验证(单测覆盖不到端到端体验):建多级分类树、任务挂分类/
   摘分类、选中父节点看到子孙任务汇总、右键删除有子孙/任务的分类后
   任务正确降级未分类且树结构正确收缩、reparent 到自己子孙下被拒绝且
   有提示、上移下移在首尾正确 no-op、重启 `dozerd` 后分类树和任务归属
   都不丢、老项目(有历史 `dozer.db`,`todos` 表没有 `category_id` 列)
   升级后能正常打开且历史任务全部显示为未分类。

## 排期备注

建议实现阶段顺序:先落 `todo_categories` 表 + `ALTER TABLE todos ADD
COLUMN category_id` 迁移 + `CategoryStore`(可独立用单测验证,不接触
协议/GUI),再落协议扩展 + `dozer-client` 七个方法,最后落 `dozer-app`
两栏布局改造(建议先接左侧树的只读展示 + 选中过滤,验证通过后再接
右键菜单的增删改/reparent/上移下移,最后接任务行分类标签 chip 和
选择器,分三步减少同时改动面)。
