# Dozer 性能优化设计

**状态:待审阅(2026-09-17 深度性能分析产出)**

## 背景

对整条数据面链路(PTY → dozerd ring/broadcast → UDS → dozer-app
`forward_events` → `App::update` → iced 渲染)做了逐段读码分析。结论:
**架构本身健康,瓶颈集中在 GUI 侧终端渲染热路径的"每帧全量重建 + 逐格
全局锁查询"**,daemon 侧基本健康,只有一处拷贝冗余。按影响从高到低列出
如下,并逐项给出"现在做 / 暂缓 / 不做"的决策与理由。

### 发现 1(高):`visible_lines()` 逐格查询 `current_scheme()` 的 `RwLock`

`term_model.rs` 的 `visible_lines()`(`term_model.rs:357`)逐行逐格调用
`fg_to_rgb`/`bg_to_rgb`,而这两个函数各自落到 `named_color_rgb`
(`term_model.rs:142`)→ `ansi16()`(`term_model.rs:63`)与 `default_fg()`
(`term_model.rs:75`),两者都调 `byteui::theme::color::current_scheme()`
(`byteui/src/theme/color.rs:147`)——**那是一个 `RwLock::read()`**。

- 100×30 网格 = 3000 格,每格前景+背景各一次 = 每帧约 6000 次
  `RwLock` 读获取/释放,随网格尺寸线性放大(全屏/大字号更多)。
- 这些值在一次 `visible_lines()` 过程中根本不会变,纯属重复取锁。

`draw()` 里(`term_view.rs:346`)还有第二个同类问题:`byteui::theme::
color::current()`(`color.rs:124`,同样 `RwLock`)在每个 run、每次光标/
滚动条绘制处被反复调用。属同类,但量级小(每帧几十次),一并顺手处理
(在 `draw` 顶部取一次 token)。

**决策:现在做。** 改动最小、纯收益、无公开签名变更,见 Task 1。

### 发现 2(高):`visible_lines()` 每帧全量重分配整个网格快照

`term_view.rs:355` 的 `draw()` 每帧调 `model.visible_lines()`,后者
`collect()` 出一个 `Vec<Vec<Cell>>`(约 `cols×rows` 个 `Cell`,每个含
`char` + 两个三元组 + 5 个 bool,≈ 24 字节;100×30 ≈ 72KB)。任何
`request_redraw`(光标闪烁、选区拖拽、悬停、面板切换)都触发一次全量
分配+释放,即使内容一帧都没变。

**决策:现在做,用"脏标记 + `Rc` 缓存"而非 `canvas::Cache`。** 选型
理由:

- 终端模型被 `App::update`(收到 `TermOutput` 消息)在 widget 树之外
  直接变更,`canvas::Cache` 的失效机制依赖 canvas widget 自己的
  `update` 被调用,那条路径收不到"内容变了"的信号,缓存会失效得不可靠。
- `TerminalModel` 被 `TermCanvas` 以不可变 `&'a TerminalModel` 借用
  (`term_view.rs:238`),`visible_lines` 不能改成 `&mut self`。用
  `RefCell<Option<Rc<Vec<Vec<Cell>>>>>` 做缓存,`visible_lines(&self)`
  返回 `Rc<Vec<Vec<Cell>>>`(干净时只是一次 refcount + 一次 borrow),
  既不破坏 `&self`,也不逃逸借用生命周期。
- 所有会改网格/视口/选区的方法(`feed`/`resize`/`scroll_display`/
  `scroll_to_bottom`/`selection_start`/`selection_update`/
  `selection_clear`)把缓存清成 `None` 即可,`None` 本身就兼当脏标记。

见 Task 2。**注意 Task 2 依赖 Task 1**(两者都改 `visible_lines` 内部,
Task 2 的 body 要调用 Task 1 改过签名的 helper)。

### 发现 3(中):文件树目录 git 状态聚合 O(N×M)

`delivery.rs:190` 的 `dir_status(dir, statuses)` 对**每个目录**遍历
**整个** `git_statuses` HashMap(`path.starts_with(dir)` 判断),而
`files/view.rs:109/200/524/679` 在渲染树的**每个目录节点**都调它一次。
N 个目录 × M 个已改动文件 = O(N×M)。大仓库(几千目录 + 数百改动)渲染
整棵树时明显卡顿。

**决策:现在做。** 预聚合一次(`rollup_dir_statuses`),把每个改动文件
的状态向上传播给它的所有祖先目录,O(M×depth) 一次算完,之后 `dir_status`
变 O(1) 查表。见 Task 3。

### 发现 4(中):`RingBuffer` 逐字节拷贝 + 整段快照拷贝

`ring.rs:23` 的 `push` 用 `self.buf.extend(data.iter().copied())` 逐字节
拷,`snapshot()`(`:41`)与 `read_from`(`:52`)是 `iter().copied().collect()`
整段拷;`server.rs:382` attach 时再把最多 1 MiB 快照 base64 一遍。

**决策:现在做,但只做"分块存储 + 预分配写"这一档。** 把 `VecDeque<u8>`
改成 `VecDeque<Box<[u8]>>`(按 chunk 存,`push` 直接 `extend` 一个 chunk、
`snapshot`/`read_from` 先 `with_capacity` 再 `write_to`),消除逐字节
`copied()`。不做真正的环形字节队列/零拷贝发送——attach 是一次性、低频
场景,收益边际递减,不值得为它重写协议面。见 Task 4。

### 发现 5(暂缓):输出 → 整窗重绘无合并

每条 `TermEvent::Output`(`hook.rs:342` 的 `forward_events` 逐条
`send_event`)都落成一条 `Message::TermOutput` → `dispatch`
(`window_events.rs:1246-1248`)里 `app.update` + **无条件**
`window.request_redraw()`,即一次完整 widget 树 diff + 重绘。交互式
agent CLI 流式输出每秒几十上百个小 chunk,每 chunk 一次整窗重绘。

**决策:暂缓。** 理由:

1. `request_redraw` 在 winit 层本来就有去抖(`setNeedsDisplay` 合并),
   多条消息在同一事件循环迭代内会被并成同一帧;真正每帧都跑的是
   `interface.update` + `draw`。
2. 发现 1 + 发现 2 落地后,单帧 `draw` 的成本大幅下降(去掉 6000 次锁 +
   72KB 分配),这个放大的**每帧**代价随之显著降低,边际收益变小。
3. 批处理会引入 8–16ms 的延迟,并牵动 `forward_events` 与 redraw 策略
   两处,风险面比 1/2/3 大。等 1/2 落地、实测帧率后,若仍不理想再作为
   独立后续计划(可考虑给 `TermCanvas` 加"输出合并窗口"或脏帧跳过)。

### 发现 6(暂缓):`layout_runs` 每 run 一次 `String` 分配

`term_view.rs:161/174/185` 每个宽字符 `cell.ch.to_string()`、每个 run
新建 `String`,`fill_cell_text` 按值收 `String`。CJK 多、风格切换多时
分配量可观。

**决策:暂缓。** 微优化,收益相对 1/2 小,且 `Run.text` 改小字符串会牵动
`layout_runs` 的测试面。等 1/2 落地并确认还不够快再考虑(`SmallString`
/`ArrayString`,或宽字符单字形用栈上 `[u8;4]` 编码)。

## 目标 / 非目标

**目标**:

1. 终端渲染热路径去掉"每帧 6000 次 `RwLock` 读"与"每帧 72KB 全量分配",
   让输出流式滚动、光标闪烁、选区拖拽、悬停等高频帧的 CPU 占用显著下降。
2. 文件树目录 git 状态聚合从 O(N×M) 降到 O(M×depth)。
3. daemon 侧 ring buffer 去掉逐字节拷贝冗余。

**非目标**:

- **不引入新依赖。**
- **不改任何可观察行为**——颜色、终端内容、目录着色、attach 快照内容
  全部与改动前一致。
- **不做发现 5(输出重绘合并)与发现 6(String 微优化)**——见各自"暂缓"
  理由,留作后续独立计划。
- **不改 UDS 协议面**(`protocol.rs` 的 JSON/base64 编码方式不动)。

## 架构

### 1. 终端逐格颜色查询去锁(`term_model.rs`)

`ansi16()`/`default_fg()` 各返回一个进程级 `RwLock` 里的值,本身不贵,
贵在**每格都调**。把解析结果在 `visible_lines()` 顶部算一次,作为参数
下传:

```rust
// 原:fg_to_rgb(color)/bg_to_rgb(color) 内部各自调 ansi16()/default_fg()
// 改:visible_lines() 顶部:
let table = ansi16();        // 一次 current_scheme() 读锁
let dfg = default_fg();      // 一次 current_scheme() 读锁
// 逐格:
fg: fg_to_rgb(cell.fg, table, dfg),
bg: bg_to_rgb(cell.bg, table, dfg),
```

`named_color_rgb`/`indexed_to_rgb`/`fg_to_rgb`/`bg_to_rgb` 四个私有 helper
都改成接收 `table: &[(u8,u8,u8); 16]` + `dfg: (u8,u8,u8)`。公开的
`ansi16_color`/`default_fg_rgb`(`pub(crate)`,预览编辑器语法高亮用)保持
原样不动。

`term_view.rs` 的 `draw` 里同样把 `byteui::theme::color::current()`
在函数顶部取一次,替换后续散落的多次调用。

### 2. `visible_lines` 脏标记 + `Rc` 缓存(`term_model.rs`)

`TerminalModel` 新增一个缓存字段:

```rust
/// 上次 `visible_lines()` 的构建结果;`None` = 脏(需要重建)。
/// 用 `Rc` 是为了 `visible_lines(&self)` 干净时能零拷贝返回一份
/// 共享句柄,不破坏 `TermCanvas` 对模型的不可变借用。
cache: RefCell<Option<Rc<Vec<Vec<Cell>>>>>,
```

`visible_lines(&self) -> Rc<Vec<Vec<Cell>>>`:

- 命中缓存:直接 `Rc::clone` 返回。
- 未命中:走 Task 1 改造后的构建逻辑,包进 `Rc`,写回缓存。

所有 mutator 在改完 `term` 状态后 `*self.cache.borrow_mut() = None`。

### 3. git 目录状态预聚合(`delivery.rs` + `files/`)

新增 `delivery::rollup_dir_statuses(statuses) -> HashMap<PathBuf,
TreeState>`:遍历每个非 ignored 的 `FileGitStatus`,沿其 `Path` 的所有
祖先目录(到根)把 `TreeState` 按优先级取 max 写进结果表;目录自身被
ignored 时置 `Ignored`(现有 `dir_status` 语义不变)。

`WorkspaceState` 新增 `dir_statuses: HashMap<PathBuf, TreeState>` 字段,
在 `StatusesRefreshed`(`files/update.rs:33`)与 `git_statuses` 一并写入。
`files/view.rs` 的 4 处 `dir_status(...)` 改成查 `ws_state.dir_statuses`
(O(1)),删除/保留 `dir_status` 的测试与旧实现(保留 `dir_status` 本身
给测试与兼容,但生产渲染不再走它)。

### 4. `RingBuffer` 分块存储(`ring.rs`)

`buf: VecDeque<u8>` → `buf: VecDeque<Box<[u8]>>`,补一个 `len: usize`
累计字节数;`push` 把入参 `data.to_vec().into_boxed_slice()` 直接塞进去;
`snapshot`/`read_from` 先 `Vec::with_capacity` 再逐 chunk
`extend_from_slice`,消除逐字节 `copied()`。公开 API 与 `SCROLLBACK_CAP`
语义不变,现有测试应全部保持绿。

## 错误处理

不适用——纯重构,无新增失败路径。

## 测试策略

- 每任务:`cargo build` + 相关 crate 的 `cargo test` + `cargo fmt --check`
  干净通过;`cargo test -p dozer-app --bin dozer` 数字与开工前一致。
- Task 2 会改动 `term_view.rs` 测试里的 3 处 `visible_lines().remove(0)`
  (改为 `[0].clone()` 或 `.first()`),`term_model.rs` 的测试只用索引/
  `.len()`,不受影响。
- Task 3 新增 `rollup_dir_statuses` 单测(与现有 `dir_status` 测试同款
  造 fixture 手法),并核对既有 `dir_status` 测试保持绿。
- Task 4 既有 `ring.rs` 测试(`push`/`snapshot`/`read_from`/逐出/超量)
  全部保持绿,另补一个"分块边界"用例(单次 push 大块 + 多次小块混合)。

## 排期备注

四个任务分属四个文件域(`term_model.rs`、`term_view.rs`、`delivery.rs`
+`files/*`、`dozerd/src/ring.rs`),互不重叠,除 Task 2 依赖 Task 1 外,
Task 3/4 可独立并行。建议单分支 `feature/perf-terminal-and-git-status`
按 Task 1→2→3→4 顺序合并,便于 review 与逐段验证。
