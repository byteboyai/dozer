# Todo 面板设计

**状态：已批准（brainstorming 会话，2026-08-06）**

**设计稿**（Figma "Dozer Phase 1 UI"，"一期主界面v2" 页，从最贴近当前代码实现的
`S1v2 主工作区` 克隆改造）：
- 主视图：https://www.figma.com/design/NXfLQp5XQk1kF7Ohls2EbX/Dozer-Phase-1-UI?node-id=120-58
- 派发选择层：https://www.figma.com/design/NXfLQp5XQk1kF7Ohls2EbX/Dozer-Phase-1-UI?node-id=127-97

## 背景

用户提出给 Dozer 加一个 todo 面板，要求"跟着项目仓库走"（像 `.dozer/goal.md` 一样 git 可追踪），
并且要能跟 agent 双向互动：GUI 能把一条 todo 派发给 agent，agent 也能直接改这个文件（改状态、
新增任务）。

现有代码已经有两条持久化路径可参考：

- `crates/dozerd`（daemon 侧结构化数据，如 `AcceptanceStore`/`ProjectStore`）走 SQLite
  （`rusqlite`，已是依赖）。
- `crates/dozer-app`（GUI 本机状态，如 `open_projects.json`/`layout.json`）走纯 JSON 文件，
  `serde_json::from_str(..).ok()` 宽容降级，不 panic。
- `.dozer/goal.md`（`crates/dozer-app/src/goal.rs`）是最接近的先例：手写解析（不引入 markdown
  库），`- [ ]`/`- [x]` 列表项当验收标准——但 `parse_goal` 对两种前缀一视同仁，真正的"是否
  达标"状态存在 dozerd 的 SQLite acceptance store 里，`goal.md` 本身只是静态声明。

本设计选择"跟着仓库走"这条路：todo 列表本身是 git 追踪的 markdown 文件，agent 直接读写它，
不引入新的 daemon 侧协议或 SQLite 表。

## 目标 / 非目标

**目标**：
1. `.dozer/todo.md`：标准两态 checkbox（`- [ ]`/`- [x]`），git 可追踪，agent 可直接编辑
   （新增任务、勾选完成）。
2. 新增 `LeftView::Todo` 面板，跟 `Files`/`Web` 平级挂在左侧图标栏。
3. 派发：用户在面板里选一条 todo，选择目标 agent tab（已有 tab 或新建），把任务文本当输入
   键入该 session（复用现有"attach 后自动键入"通路）。
4. 面板展示三态：待办 / 进行中 / 完成——"进行中"是**派生态**（未勾选 + 已派发 + 目标
   session 仍存活），不写回文件，不需要 agent 维护一个非标准的第三态语法。
5. 文件变化的同步靠轮询 mtime，只在 Todo 面板可见时才轮询；GUI 侧写入（勾选/新增）用定点
   单行替换，不整份重写，把与 agent 并发写入的冲突面压到最小。
6. 面板顶部一条工具栏：按状态筛选（全部/待办/进行中/完成）+ 关键字搜索框，纯 GUI 侧的
   内存过滤，不碰文件、不碰 sidecar、不改协议。
7. 每条任务可选带"计划时间"（用户在面板里手动设，agent 不写不读）和"完成时间"（勾选变
   完成的那一刻 GUI 自动盖章）；两者都是 GUI 本地记录，不写进 `.dozer/todo.md`。

**非目标**：
- **不引入 markdown 解析库**——沿用 `goal.rs` 的手写宽松解析风格。
- **不引入文件监听依赖（`notify`）**——用轮询，代码库目前没有 fs-watch 基础设施，新增一个
  纯为这个功能服务的依赖不划算。
- **不把"进行中"写进文件**——避免非标准 checkbox 语法（`- [~]` 之类），也避免"agent 忘记
  维护中间态"这个不可控的失败模式。
- **不做优先级/负责人等富元数据**——计划时间/完成时间已经在 v1 范围内（见目标 7），但两者
  都是 GUI 本地记录、agent 不可见不可改，不算"写进 markdown 的富元数据"；优先级、负责人
  依然 YAGNI。
- **不做跨项目聚合视图**——每个项目各自的 `.dozer/todo.md`，面板只显示当前激活项目的。
- **不做写入冲突的合并/diff UI**——冲突（GUI 要改的那一行已被 agent 改掉）直接静默放弃这次
  写入 + 强制重读，不做更复杂的三方合并。
- **不给 markdown 行发明稳定 id**——派发记录用任务文本内容哈希做 key，代价是"改了任务文字会
  丢派发记录"，接受这个限制（详见下文）。

## 关键语义确认（brainstorming 会话定案）

- 派发流程是"用户先选 agent/tab，再派发"——不是自动发给当前聚焦的 tab，也不是每次都强制新建。
- 三态模型：待办 / 进行中 / 完成。"进行中"完全由 GUI 运行时推算，不落盘。
- 同步策略：定时轮询 `.dozer/todo.md` 的 mtime，复用 `App::advance_hover_anims` 同款"自驱
  redraw、没事就不排下一拍"范式，但触发条件是"当前激活项目的 `left_view == LeftView::Todo`"，
  不是"有动画在跑"。
- GUI 写入用**单行定点替换**，不是整份文件重写；找不到要改的原始行（说明文件已被并发改过）
  时静默放弃、触发强制重读，不报错不弹窗。
- 派发记录（todo 行 ↔ 目标 tab/session）存在 GUI 本地 JSON sidecar 里，不进 git，不影响
  `todo.md` 本身的格式。
- 筛选（全部/待办/进行中/完成）与关键字搜索是纯前端过滤，作用在已经解析+推导好状态的
  列表上，不引入新消息之外的状态——切换筛选/输入关键字不触发重新解析文件、不写盘。
- 计划时间只能用户在面板里手动设/改，agent 不写这个字段；完成时间由 GUI 在勾选框从未完成
  变完成的那一刻自动盖章，用户/agent 都不用手动填。两者都不进 `.dozer/todo.md`，跟派发记录
  一样存进 GUI 本地 sidecar——用户取消勾选（完成→待办）时，完成时间一并清空，避免一个
  待办任务身上挂着一个陈旧的"完成于"时间戳。

## 架构与数据流

### 1. `todo.rs`：文件格式与解析

新增模块，结构上镜像 `goal.rs`：

```rust
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

pub fn todo_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("todo.md")
}

/// 宽松解析：只认一级 `- [ ]`/`- [x]` 列表项，其余行（标题、正文、多级
/// 缩进）一律忽略，不因为格式意外而失败。文件不存在/为空 → 空列表，
/// 不是 `Option`（跟 `parse_goal` 不同——todo 没有"整份文件代表一个目标"
/// 这种"要么有要么没有"的语义，空列表本身就是合法状态）。
pub fn parse_todo(md: &str) -> Vec<TodoItem>
```

同文件里放两个纯函数写入辅助（供 GUI 侧勾选/新增调用）：

```rust
/// 在 `content` 里找到与 `old_line` 逐字节相同的一行，替换成 `new_line`。
/// 找不到（文件已被并发改过）返回 `None`，调用方按"冲突，放弃这次写入,
/// 强制重读"处理，不是错误。
pub fn replace_todo_line(content: &str, old_line: &str, new_line: &str) -> Option<String>

/// 在最后一个 `- [ ]`/`- [x]` 行之后追加一条新任务；纯追加不依赖"找到
/// 匹配行"，冲突面比替换小。
pub fn append_todo_item(content: &str, text: &str) -> String
```

### 2. `LeftView::Todo` 面板挂载

```rust
pub enum LeftView {
    Files,
    Web,
    Todo,
}
```

`RailButton` 加 `LeftTodo` 变体，`HoverId::Rail(RailButton::LeftTodo)` 走现有 hover 动画机制，
渲染方式与 `LeftFiles`/`LeftWeb` 的 rail 按钮逐字节一致（同一段代码稍作参数化，不新增分支
逻辑）。

`todo_pane` 渲染函数（新，挂在 `left_view == LeftView::Todo` 分支）：每条 `TodoItem` 一行，
勾选框（点击触发 `Message::TodoToggle(idx)`）+ 任务文本 + 一个"派发"图标按钮（触发
`Message::TodoDispatchOpen(idx)`），面板底部一个"＋新增"输入行（复用项目树"新建文件"那种
内联编辑框模式）。

### 3. 派发流程与本地元数据（sidecar）

```rust
/// GUI 本地任务元数据，`dozer_core::paths::config_dir()/todo_meta.json`，
/// 跟 `open_projects.json` 同一挂靠模式：纯运行时缓存，不进 git，读失败
/// （不存在/损坏）一律回落空 map，不 panic（`serde_json::from_str(..).ok()`,
/// 同 `open_projects::load_from` 的既有惯例）。三个字段都是可选的、互相独立
/// 的 GUI 侧记录，agent 不读不写——这个结构体本身就是"todo.md 之外，GUI
/// 私有的那部分状态"的完整清单。
pub struct TodoTaskMeta {
    pub dispatch: Option<DispatchRecord>,
    /// 用户手动设的计划时间，自由文本（v1 不做日期合法性/格式校验，交给
    /// 面板里的输入控件自己保证大致合理）。
    pub plan_date: Option<String>,
    /// 勾选框从未完成变完成的那一刻由 GUI 写入；取消勾选时清空（见"关键
    /// 语义确认"）。
    pub completed_at: Option<SystemTime>,
}
pub struct DispatchRecord {
    pub tab_id: usize,
    pub dispatched_at: SystemTime,
}
pub type TodoMetaState = HashMap<ProjectId, HashMap<u64, TodoTaskMeta>>;

/// key 用任务文本 trim 后算哈希，不给 markdown 行发明稳定 id——代价是
/// 改了任务文字会跟丢这条的全部本地元数据（派发记录+计划时间+完成时间），
/// v1 接受。
fn todo_line_key(text: &str) -> u64
```

点击"派发"弹出一个选择层（样式复用 `agent_picker_popup`）：列出当前项目存活的 agent tab +
"新建"选项。选中后：
- 已有 tab → 复用 `client.write`（同 `picker_launch_command` 的"attach 后自动键入"通路），
  把任务文本当输入写进去。
- "新建" → 走现有 agent 选择菜单，新建后自动键入任务文本（同一条 `spawn_new_tab` 路径，
  `launch` 参数已经支持自定义初始命令，见 `picker_launch_command`）。
- 两种情况都往对应任务的 `TodoTaskMeta.dispatch` 记一条 `{tab_id, dispatched_at}`（不影响
  同一条记录里已有的 `plan_date`/`completed_at`）。

### 4. 三态推导（纯函数，可单测）

```rust
pub enum TodoState { Pending, InProgress, Done }

/// `done` 为真直接 `Done`；否则看 sidecar 有没有这条的派发记录，记录存在
/// 且目标 tab 的 `AgentState`/`alive` 仍显示存活 → `InProgress`；否则
/// （没派发过，或派发目标已经退出）→ `Pending`。
pub fn todo_display_state(
    item: &TodoItem,
    dispatch: Option<&DispatchRecord>,
    target_alive: bool,
) -> TodoState
```

调用方传 `meta.dispatch.as_ref()`（`meta: &TodoTaskMeta`，取自 `TodoMetaState`）；`plan_date`/
`completed_at` 不参与这个推导，三态只看完成勾选 + 派发存活，跟计划/完成时间显示是两件事。

### 5. 轮询同步

`left_view` 是 `App` 上的字段（随激活项目切换时从该项目的 `ShellLayout` 同步进来，不是
`Workspace` 的字段），所以轮询状态（上次已知 mtime）也记在 `App` 上，跟着 `left_view` 走。`main.rs` 的 `about_to_wait` 新增一个独立分支（不影响现有 blink/hover 判断，
三者各自决定是否要排下一拍）：当前激活项目 `left_view == LeftView::Todo` 时，排一拍
`TODO_POLL_INTERVAL`（约 1s）唤醒；唤醒后 `stat` mtime，变了才 `read_to_string` + `parse_todo`
+ 请求重绘，没变就是一次系统调用，忽略不计的开销。面板不可见时不排这个唤醒，回到现有的
"没事就 `Wait`"省电路径。

### 6. GUI 写入路径

用户勾选/新增任务时：读当前文件内容 → 用 `replace_todo_line`/`append_todo_item` 算出新内容
→ 写回。`replace_todo_line` 返回 `None`（原始行已被并发改过）时：不写、丢弃这次操作、强制
触发一次重读（走跟轮询命中同一条刷新路径），不报错不弹窗——下一帧 UI 展示的是 agent 改完后
的最新状态。

### 7. 筛选与搜索

```rust
pub enum TodoFilter { All, Pending, InProgress, Done }
```

面板顶部工具栏：一个分段控件（全部/待办/进行中/完成，对应 `TodoFilter`）+ 一个搜索输入框。
两者都是纯前端状态（`App`/`Workspace` 上各加一个字段，不持久化，重启回落到
`TodoFilter::All` + 空关键字），渲染 `todo_pane` 时先对完整列表跑一遍
`todo_display_state`（见第 4 节）得到每条的三态，再依次按 `TodoFilter`（相等匹配）和关键字
（对 `TodoItem.text` 做大小写不敏感的子串匹配，空关键字＝不过滤）筛一遍，最后渲染筛完的
结果。不影响 `TodoItem`/`parse_todo`/写入路径——筛选只作用于"已经解析+推导好的内存列表"
这一层，跟文件、跟 sidecar 都无关。

### 8. 计划时间 / 完成时间

- 计划时间：面板里点一条待办/进行中任务旁的时间文字，进入内联编辑（同项目树"重命名"的
  内联编辑框模式），输入内容原样存进 `TodoTaskMeta.plan_date`；不做日期格式校验，用户想写
  "08-10"还是"下周三"都行——这是纯提醒性质的自由文本，不参与任何排序/分组之外的语义计算
  （v1 也不做按计划时间排序，只是显示）。
- 完成时间：`Message::TodoToggle(idx)` 的处理逻辑里，勾选框从 `false` 翻到 `true` 的那一
  刻，顺带把 `TodoTaskMeta.completed_at` 设成 `SystemTime::now()`；翻回 `false`（用户手滑
  取消勾选）时清空该字段。全程 GUI 侧完成，不涉及文件读写、不涉及轮询。
- 两者渲染：待办/进行中行在任务文本右侧、派发按钮左侧插入"计划 <plan_date>"（`plan_date`
  为 `None` 时不渲染这段，不留空白占位）；完成行渲染"完成于 <格式化的 completed_at>"取代
  派发按钮的位置（完成行本来就没有派发按钮）。

## 错误处理

- `.dozer/todo.md` 不存在 → `parse_todo("")` 返回空列表，面板显示"还没有 todo"空态。
- 文件里有解析不了的行 → 跳过，不整体失败（同 `parse_goal` 的宽容风格）。
- `todo_meta.json` 损坏/不存在 → 当空 map，派发记录/计划时间/完成时间全部丢失但不影响面板
  核心功能（勾选/查看/新增照常工作，只是"进行中"会暂时全部回落成"待办"、时间显示为空，
  下次派发/勾选/编辑计划时间时重新落盘即可恢复）。
- GUI 写入冲突（`replace_todo_line` 返回 `None`）→ 静默放弃 + 强制重读，不是错误路径。
- 派发目标 tab 在选择过程中被关闭 → 走现有"session 已不存在"的降级路径（`tracing::warn!` +
  no-op），不新造错误处理机制。
- 磁盘写入失败（权限/磁盘满）→ `tracing::warn!` 记录，内存态保持乐观（用户看到的勾选状态
  可能跟磁盘不一致直到下次成功写入或轮询覆盖），不弹阻断性错误——对齐 `acceptance_reject`
  现有的降级级别。

## 测试策略

- `parse_todo`：空文件、纯标题无任务、`[ ]`/`[x]` 混合、夹杂无关正文/多级缩进（应被忽略）。
- `replace_todo_line`：命中替换、未命中返回 `None`、文件里有多行相同文本时只替换第一次匹配
  （已知限制，文档里写明）。
- `append_todo_item`：空列表追加、已有若干条后追加（验证插入位置在最后一条任务行之后）。
- `todo_display_state`：`done=true`（不管 sidecar）→ `Done`；`done=false` + 无派发记录 →
  `Pending`；`done=false` + 有记录 + 目标存活 → `InProgress`；`done=false` + 有记录 + 目标已
  退出 → `Pending`（组合表覆盖四种情况）。
- `todo_line_key`：相同文本（含前后空白差异，trim 后应相等）产生相同 key，不同文本产生不同
  key（不要求无碰撞，只要求"实践中够用"，同现有其它哈希用途的验收标准）。
- `TodoMetaState` 的 load/save：镜像 `open_projects.rs` 现有的
  `load_from_missing_file_returns_default`/`load_from_corrupt_file_returns_default` 两个测试
  模式；额外覆盖"`TodoTaskMeta` 三个字段互相独立"——只设 `plan_date` 不影响 `dispatch`，
  反之亦然。
- 筛选/搜索的组合逻辑：`TodoFilter` 四个取值 × 有/无关键字命中的组合表（纯函数，输入
  完整列表+筛选条件，输出筛完的子集，不涉及 iced/GUI）。
- "勾选变完成时 `completed_at` 被设为当前时间；取消勾选时 `completed_at` 被清空"——这条
  状态转换逻辑（`Message::TodoToggle` 里的一小段）拆成纯函数单测，不用起 GUI。
- 轮询触发/停止、派发弹层的真实视觉效果、计划时间的内联编辑交互、"进行中"状态随 agent
  会话退出而回落成"待办"——这几条留给人工验收（`cargo run -p dozer-app` 目测），同
  P1L/Agent 域面板的既有惯例，不做自动化 GUI 测试。

## 依赖变更

无新增依赖（不引入 `notify`，不引入 markdown 解析库）。
