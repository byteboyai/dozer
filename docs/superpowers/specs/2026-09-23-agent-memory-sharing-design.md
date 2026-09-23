# Agent 记忆共享模块(project 级、SQLite 权威、专属 UI)

**状态:已批准(brainstorming 会话,2026-09-23)**

## 背景

Dozer 目前对"agent 记忆"完全是旁观者角色:`crates/dozer-app/src/extensions/project/links.rs`
的 `discover_memory`/`discover_memory_in` 只是自动发现各 agent **自己**存在自己
home 目录下的记忆/指令文件——项目根内的 `claude.md`/`agents.md`/`.claude`/
`.cursor` 等,以及仓库外的 `claude_project_dir/memory`、`codebuddy_project_dir`、
`opencode_project_dir` 三个目录——展示成项目面板"Agent 记忆"区里的一份虚拟链接
列表(`LinksState::memory`),点文件走 `Message::OpenLink` 进标准 Preview 管线
查看/编辑,点目录只是展开浏览。这套机制**不拥有任何记忆数据**,纯粹是"帮用户
找到各家分散的记忆文件放在哪";不同 agent 之间的记忆互不相通,写在 CLAUDE.md
里的东西 CodeBuddy/OpenCode 读不到,反之亦然。

本设计要解决两个目标:

1. 让接入 Dozer 的多个 agent CLI(Claude Code / CodeBuddy / Codex / OpenCode /
   v8agent,`dozer-mcp` 覆盖的这五家——见下方"边界声明")之间共享同一份记忆,
   写入后**送达有保证**(不是"碰巧都读同一个文件夹"这种尽力而为)。
2. 让人类用户能看到这份记忆里具体有什么、每条记忆是怎么演变过来的(谁在什么
   时候改了什么),并能做少量修改。

调研过程中对比了两个开源项目(mem0、basic-memory),关键借鉴点已经在
brainstorming 阶段讨论过:

- **basic-memory** 的"文件是唯一真相源、SQLite 只是派生索引"哲学很贴近 Dozer
  一贯的裁决(`agent_paths.rs` 里"dozerd 读别的 agent 自己格式目录、不拥有
  存储"是同一个哲学),但**不采用**它的文件方案——本设计需要的"多 agent 并发
  写入 + 送达保证 + 审计历史"三个要求放在一起,文件多写者场景会重新引入
  Dozer 已经在 Preview 里解决过一次的磁盘竞态问题(T10),而审计历史文件本身
  也不天然提供,等于两头复杂度都占了。最终选择 SQLite 权威 + 专属 UI(方案
  A),放弃仿 basic-memory 的文件权威方案(方案 B)。
- **mem0** 的"一个共享核心库 + 每个 agent 一份瘦 manifest/wrapper,共用同一套
  MCP 工具"这个形状,对应到 Dozer 就是`dozer-mcp` 现有的 Todo 工具模式——本
  设计直接照抄这个已验证的模式,不是重新发明。

**已确认的现有机制,本设计直接复用,不重新发明**:

- `crates/dozerd/src/todo.rs` 的 `TodoStore`:project_id 作用域、
  `CREATE TABLE IF NOT EXISTS` 幂等迁移、单文件 `dozer.db`、`Mutex<Connection>`
  ——`MemoryStore` 照抄这套模式。
- `crates/dozer-mcp/src/server.rs` 的工具形状:`#[tool_router(server_handler)]`
  + `Parameters<T>` 入参 + `CallToolResult::structured` 出参,`resolve_project_id`
  已经把 `session_id → project_id` 的解析抽成了公共方法,新工具直接复用。
- `dozer_core::protocol::SessionInfo.agent: AgentKind` 已经带着当前会话是哪个
  agent(`Claude`/`Codebuddy`/`Opencode`/`Codex`/`Goose`/`Aider`/`V8agent`/
  `Unknown`)——`resolve_project_id` 拿到的 `session` 里就有这个字段,写入记忆
  时的 `created_by`/`updated_by` 归属信息直接从这里取,不需要新的身份传递
  链路。
- `crates/dozer-mcp/src/install.rs::config_path_for` 现有的四家安装器
  (claude/codebuddy/codex/opencode)+ v8agent 自己 CLI 硬编码挂载 `dozer-mcp`
  ——新工具随 `dozer-mcp` 二进制一起生效,不需要改安装器。

## 目标 / 非目标

**目标**:

- `dozerd` 新增 project 级共享记忆存储,任何连了 `dozer-mcp` 的 agent CLI 写入
  后,其他连了同一个 `dozerd` 的 agent CLI 立即能读到一致内容。
- 项目面板提供一个专属"共享记忆"UI:列表 + 详情 + 修改历史时间线,支持人工
  编辑/新建/删除。
- 记忆条目带审计轨迹:谁(agent 或用户)在什么时候把内容改成了什么。

**非目标(本次不做,留白不是遗忘)**:

- 跨项目的全局记忆(只做 project_id 作用域,和 Todo 一致)。
- 语义搜索/向量检索(`search_memories` 之类)——记忆量级小时用不上,以后需要
  再加。
- 编辑冲突检测/乐观锁——v1 后写覆盖先写,不做版本号校验。
- agent 侧删除权限——`dozer-mcp` 不提供 `delete_memory` 工具,删除只能人工在
  UI 里做。
- 现有 `discover_memory`/`LinksState::memory` 的数据迁移——这套机制**整体
  下线**,原 `.dozer/links.json` 里 `memory` 字段的既有数据(含用户手动
  `dismissed` 过的路径)不再被读取,不做迁移脚本。
- goose/aider 覆盖面——这两家目前没有 `dozer-mcp` 工具通道(只有 hook/journal
  单向摄取),本设计不新增它们的 MCP 安装支持,是已知覆盖缺口。

## 边界声明("强制共享"到底保证了什么)

"强制"只落在**送达一致性**上:写入 `write_memory` 的内容,所有连了
`dozer-mcp` 的 agent CLI 调 `list_memories`/`get_memory` 都能读到同一份最新
结果(因为大家读写的都是同一个 `dozerd` 拥有的 SQLite,没有本地文件缓存/
轮询延迟这类不一致来源)。

"强制"**不**代表、也做不到:

- 阻止某个 agent CLI 继续使用它自己原生的记忆机制(比如 Claude Code 自己的
  自动记忆系统)——Dozer 管不到宿主 CLI 内部的行为,只能通过 MCP 工具描述文案
  引导"优先读写这里",这是纯软性引导,不是硬约束。
- 覆盖 goose/aider 这两家(见"非目标")。

## 架构与数据流

### 数据模型(`dozerd`)

新增 `crates/dozerd/src/memory.rs`(命名避免和既有 `mod` 冲突,不叫
`memories.rs` 是因为要跟 `MemoryStore` 类型名对齐,参考 `todo.rs`/
`TodoStore` 的命名习惯),两张表,同一个 `dozer.db`:

```sql
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    created_by TEXT NOT NULL,
    updated_ms INTEGER NOT NULL,
    updated_by TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS memories_project_title
    ON memories(project_id, title);

CREATE TABLE IF NOT EXISTS memory_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    changed_ms INTEGER NOT NULL,
    changed_by TEXT NOT NULL,
    change_kind TEXT NOT NULL,       -- 'created' | 'updated' | 'deleted'
    title_snapshot TEXT NOT NULL,
    kind_snapshot TEXT NOT NULL,
    description_snapshot TEXT NOT NULL,
    body_snapshot TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS memory_history_memory
    ON memory_history(memory_id, changed_ms);
```

要点:

- `kind` 是自由 TEXT,不做 DB 级枚举约束。默认值集合建议复用
  `user`/`feedback`/`project`/`reference` 四分类(跟这次 brainstorming 过程中
  引用的 Claude 自身记忆系统同一套分类,对"AI 协作场景下该记什么"这件事已经
  验证过是够用的),但其他 agent 塞别的字符串也不报错。
- `(project_id, title)` 唯一索引承载"按标题 upsert"的语义——`write_memory`
  靠这个索引判断是新建还是更新,不需要调用方先查 id。
- `memory_history` 存**改动后的完整快照**而不是 diff:实现简单,查询时间线不
  需要额外拼接逻辑;以后要做相邻快照的行级 diff 高亮,在读出两条快照后临时算
  即可,不需要现在就写 diff 算法。删除一条记忆时也在这张表里追加一条
  `change_kind = 'deleted'` 的快照(内容是删除前的最后状态),`memories` 表里
  的原行物理删除——历史表不因为主表删除而跟着清空,历史轨迹要能在记忆被删除
  后仍然可查。

`MemoryStore` 方法面(镜像 `TodoStore` 的形状):

```rust
impl MemoryStore {
    pub fn new(db_path: &Path) -> Result<Self>;
    pub fn list(&self, project_id: i64) -> Result<Vec<MemoryInfo>>;      // 不含 body
    pub fn get(&self, project_id: i64, id: i64) -> Result<Option<MemoryDetail>>; // 含 body + 最近历史
    pub fn write(&self, project_id: i64, title: &str, kind: &str,
                 description: &str, body: &str, actor: &str) -> Result<MemoryDetail>; // upsert
    pub fn delete(&self, project_id: i64, id: i64, actor: &str) -> Result<()>;
    pub fn history(&self, project_id: i64, id: i64, limit: i64) -> Result<Vec<MemoryHistoryEntry>>;
}
```

### `dozer-mcp` 新工具

新增到 `crates/dozer-mcp/src/server.rs`,和现有 Todo 工具并列:

- `write_memory(title, body, kind, description)` —— 按 `(project_id, title)`
  upsert;`actor` 从 `resolve_project_id` 顺带拿到的 `session.agent.as_str()`
  推导(`AgentKind` 已有 `as_str()`,见 `protocol.rs:41`)。工具描述明确写:
  "记忆优先读写这里,不要用你自己本地的记忆机制;写入前建议先用
  `list_memories` 查一遍有没有同名条目可以更新,避免重复记忆"。
- `list_memories()` —— 返回 `id/title/kind/description/updated_ms/updated_by`,
  不带 `body`。
- `get_memory(title_or_id)` —— 返回完整 `body` + 最近若干条 `memory_history`。
  入参接受标题或数字 id 两种形式(agent 一般只知道标题,不知道 id)。

**没有** `delete_memory` 工具(见"非目标")。

### 归属信息

`created_by`/`updated_by`/`changed_by` 统一存 `AgentKind::as_str()` 的结果
(`"claude"`/`"codebuddy"`/...),人工在 UI 里操作时存字面量 `"user"`。UI 渲染
时用 `AgentKind::label()` 同款映射转成人类可读名字+既有的 agent 识别色(复用
`workspace.rs::agent_dot_color`,不新造配色),`"user"` 单独给一个中性展示,不
占用四个 agent 色里的任何一个也不用金色(金色是甲方动作专属,这里的"用户
在 Dozer 里编辑记忆"不算甲方对 agent 下达动作,是治理层的旁路操作,用中性色
即可,具体取色留给实现阶段的 UI 细节)。

### 项目面板 UI 改造

**整体替换**现有"Agent 记忆"区(`crates/dozer-app/src/extensions/project/`
下 `links.rs`/`view.rs` 里 `LinkTarget::Memory` 相关代码路径):

- 不再跑 `discover_memory`/`merge_rediscovered` 的 Memory 分支,`LinksState`
  的 `memory: Vec<LinkEntry>` 字段和相关 UI 渲染整体删除(`Docs` 分支不受
  影响,文档虚拟链接功能保留)。
- 新增一个共享记忆子视图:
  - **列表**:每行 title + kind 徽标 + description + "上次由 {actor} 于
    {relative_time} 更新",点击进详情。
  - **详情**:标题/kind/description/body 四个可编辑字段的表单 + 保存按钮;
    下方历史时间线(时间 + 改动方 + 改动类型),点一条历史看该次快照的只读
    全文。
  - **操作**:详情页保存(编辑现有条目)、列表页"+"新建(同 Todo 面板的新建
    交互)、详情页删除(需要一次二次确认,因为是不可逆的"从共享池移除",
    但历史记录仍保留可查)。
- 新旧命名:如果`extensions/` 目录未来要走扩展化改造(参考已完工的
  Todo/Files/Usage 扩展化试点),这块新 UI 建议直接按扩展化模式写(独立
  `WorkspaceState` + 消息前缀 + emit 回调),不要先写成耦合在
  `project/view.rs` 里的老形态再迁移一遍——但这不是本设计的硬性要求,具体
  落地方式留给实现计划阶段判断当前 `project` 扩展本身是否已经是扩展化形态
  (需要在写实现计划前读一遍 `crates/dozer-app/src/extensions/project/`
  当前状态确认)。

### `dozerd::server::serve` 参数膨胀问题(顺带改)

`crates/dozerd/src/server.rs::serve` 当前已有 12 个位置参数(`registry`/
`projects`/`bookmarks`/`code_health`/`transcripts`/`session_summaries`/
`backfill_registry`/`todos`/`categories`/`ide_lock_dir`/`in_flight`/
`socket`),其中多个是相邻同类型的 `Arc<XStore>`,已经超过 CLAUDE.md 里"≥7
个参数、多个同类型相邻"要触发具名字段参数结构体这条裁决的阈值。本设计要新增
第 13 个(`Arc<MemoryStore>`),**不能**再无脑往参数列表后面加一个——实现计划
里需要把这些 `Arc<XStore>` 归到一个具名字段的 `Stores` 结构体(或类似命名),
`socket`/`ide_lock_dir`/`in_flight` 这几个非 store 性质的参数是否一并归进去,
留给实现计划阶段判断,但至少所有 `Arc<XStore>` 必须归一个结构体,不能继续
平铺。这算是本设计顺手清理的既有代码问题,不是范围蔓延。

## 错误处理

- `write_memory` 的 `title`/`body` 为空:参考 `add_todo` 现有行为(未做非空
  校验,允许空文本),本设计**不**新增校验,保持和 Todo 工具一致的宽松输入
  策略——如果空标题导致 UI 体验差,留给实现阶段按 Todo 的实际处理方式对齐,
  不单独发明一套规则。
- `get_memory`/`delete_memory`(UI 内部用,不是 MCP 工具)传入不存在的
  `id`:返回 `Ok(None)`/明确的"not found"错误,不 panic——照抄
  `TodoStore` 现有的 `Result<Option<T>>`/显式 err 模式。
- 会话未归属项目(`resolve_project_id` 返回 `Err`)时,三个新 MCP 工具和现有
  Todo 工具一样直接把错误透传成 `McpError::internal_error`,不做特殊兜底。

## 测试策略

- `crates/dozerd/src/memory.rs` 单测(照抄 `todo.rs` 里 `#[cfg(test)]` 模块的
  组织方式):
  - upsert 语义(同标题第二次写入是更新不是新建,`memories` 表只有一行)。
  - 每次 `write`/`delete` 都在 `memory_history` 追加一条快照,字段内容正确。
  - `list` 不返回 `body`,`get` 返回完整 `body` + 历史。
  - project_id 隔离(不同项目的同名 title 互不影响)。
- `crates/dozer-mcp/tests/memory_tools.rs`(照抄 `tests/todo_tools.rs` 的
  daemon 拉起方式,新增 `MemoryStore` 到 `start_daemon` 的 store 列表里):
  - `write_memory` 首次调用创建、二次调用更新同一条。
  - `list_memories`/`get_memory` 返回结构和字段裁剪(list 不含 body)符合
    预期。
  - `resolve_project_id` 失败路径(会话未归属项目)时三个工具都返回
    `internal_error`。
- UI 部分(项目面板共享记忆视图)只能人工在真实 GUI 里验收:新建/编辑/删除
  的交互闭环、历史时间线渲染、旧"Agent 记忆"区确认已经整体消失且没有残留
  的 `LinkTarget::Memory` 分支代码或死代码警告。

## 排期备注

本设计范围内的实现顺序建议:①`dozerd` 数据模型 + `MemoryStore`(含单测)→
②`dozer-mcp` 三个新工具(含集成测试)→③`serve()` 参数结构体重构(必须在
②接入 `MemoryStore` 到 `serve()` 之前做,否则又是往已经超标的参数列表上加
一个)→④项目面板 UI 改造(替换旧区块)。前三步互相有依赖,不适合并行拆给
多个 subagent;第④步依赖前三步全部完成才能接上真实数据,也不宜提前并行。
