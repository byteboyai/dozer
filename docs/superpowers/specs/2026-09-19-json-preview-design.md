# JSON Preview（Tree Viewer）设计

## 背景与动机

当前 `.json`/`.jsonl`/`.ndjson` 落在 `preview::is_editable_extension` 的原生代码编辑器路径（`crates/dozer-app/src/preview/native_editor.rs`），JSON 语法高亮的纯文本展示——没有结构化折叠、没有类型标注、超大文件（数十万行的单个 JSON blob 或百万行 jsonl 导出）打开会全量读进 `CodeView` 缓冲区，没有任何封顶/虚拟化。

本设计借鉴 [`2026-09-19-tabular-viewer-design.md`](./2026-09-19-tabular-viewer-design.md)（Excel/CSV 表格预览）刚验证过的一套原则，但**不是照搬其实现**——中途一次代码审查 + 用户新提的"1GB 基准、必须打开快"要求，把 tabular 的加载路径从"同步全量加载"改造成了"后台线程 + 流式/够数即停解析 + 统一 loading 动画"，这份 JSON 设计从第一天就按改造后的最终形态设计，不重走"先同步再补救"的弯路。借鉴的具体原则：

1. **虚拟化渲染**：只画可视窗口内的节点，帧开销与总数据量无关。
2. **够数即停 / 惰性物化**：不为了拿到一个"完整"的数据结构而读完整个文件；封顶后近似统计，不追求精确计数。
3. **加载离开 UI 线程**：解析用 `spawn_blocking` 起后台线程,配统一 `loading_hint` 动画,绝不卡住 iced 事件循环。
4. **只读优先**：预览渲染,不提供编辑入口(CLAUDE.md 核心原则);深度操作(JSONPath 查询、编辑、校验)留给后续 MCP tools 暴露给 AI agent,这次不做。

JSON 与 xlsx 有一个本质差异,影响了这次的核心技术选型:JSON 是**树形**结构而不是**表格**——数据本身没有"行"的概念,天然适合"默认折叠、点开才看"的树 UI,这恰好是"惰性物化"最自然的落地场景(比 tabular 硬套"行数封顶"更贴合数据本身的形状)。

## 目标 / 非目标

**目标：**

1. 新增一个 iced 原生的 JSON Tree Viewer,把 `.json`/`.jsonl`/`.ndjson` 从纯文本代码编辑器路径改为委托给它,默认展示可展开/折叠的树。
2. **Tree / 原始文本双视图切换**——与 tabular 完全接管 `.xlsx`/`.csv` 不同,JSON 本身是纯文本,用户可能需要复制/查看原始格式;顶部提供切换按钮,在 Tree 视图与现有的原生代码编辑器(JSON 语法高亮纯文本)之间切换,两个视图共享同一份已加载数据,来回切不重新加载文件。
3. **高性能,支持 1GB 级文件**:基于第三方高性能 JSON 库的"惰性/on-demand"能力,只解析/物化用户实际展开的节点;未展开的子树只记"这里有个 Object/Array,大概多少个子项"这类形状信息,不深入解码其内容。canvas 虚拟化渲染,只画可视窗口内的行。
4. `.jsonl`/`.ndjson` 支持:按行流式读取,每行是一个独立的懒加载根节点,读满行数封顶即停(同 CSV 的"够数即停")。
5. 类型标注:节点按 JSON 类型(object/array/string/number/bool/null)显示,object 的 key 序,array 的下标,叶子值做展示裁剪(超长字符串截断)。
6. 配色对齐 ByteBoy2077,深浅色自动跟随全局 `set_scheme`。

**非目标(一期裁掉):**

- 编辑 / 写回 JSON(预览即渲染,不提供直接编辑入口)。
- JSONPath / jq 风格查询、按 key 全文搜索(留给未来 MCP tools)。
- JSON Schema 校验、类型推断之外的语义理解。
- 排序 / 过滤 / 节点拖拽重排。
- 单元格/节点复制到剪贴板之外的编辑类交互(纯文本视图本身可选中复制,已经覆盖"要原文"的诉求,不用在树上再做一套)。
- 除 `.json`/`.jsonl`/`.ndjson` 外的近似格式(如 JSON5、JSONC 带注释)——只认标准 JSON。
- 对象/数组封顶之上的分页浏览(超出封顶的子项只提示"还有更多",不提供"加载下一页")。

## 关键技术决策与风险

### 解析器:sonic-rs 的 on-demand 模式

选用 [`sonic-rs`](https://crates.io/crates/sonic-rs)(ByteDance,SIMD 加速,支持 stable Rust,x86_64/aarch64):它是目前唯一同时满足"够快"与"能只解析被访问路径、跳过未访问容器/字符串"两个条件的 Rust JSON 库——

- `simd-json`:同样 SIMD,但内部先建 tape 再转 Rust 结构,是全量 DOM 语义,没有"跳过未访问内容"的能力,基准测试比 sonic-rs 慢。
- `struson`:纯流式(SAX 风格)读写,语义上最贴近"逐步读、不整篇进内存",但其自身文档明确写"性能还不够好,仍是实验阶段",不满足"高性能"要求。
- `serde_json`:标准选择,全量 DOM 或需要手写 `Visitor` 才能流式,没有 SIMD 加速,不是这次要的"高性能"选项。

**已知风险,需要在实现前用一个 spike 验证(计划里是第一个 Task)**:SIMD on-demand 解析器(sonic-rs 与其对标的 C++ simdjson 同源)通常是**前向游标**语义——按文档顺序访问字段/数组项,一旦"越过"某个值,一般不能无代价地跳回去。JSON Tree UI 需要支持"用户任意顺序展开任意节点"(先展开第 5 个数组元素,再回头展开第 1 个的某个 key),这与前向游标天然冲突。

Spike 要回答两个问题:

1. sonic-rs 是否提供"记录当前字节偏移、之后凭偏移直接跳回并继续读"的能力(多数 on-demand 解析器把"跳过一个值"实现成廉价的括号匹配,而不需要真正解码,如果"跳到某偏移继续解析"同样廉价,随意顺序展开就没问题)。
2. 能否在不解码子节点值的前提下,廉价拿到一个 object 的 key 列表 / array 的长度(用于"未展开时也要显示有几个子项"这个 UI 需求)。

**回退方案(spike 失败时)**:退回"sonic-rs 全量 DOM 解析(`sonic_rs::Value`)+ 只在渲染时懒建 widget"——一次性 SIMD 解析整份文档到内存(SIMD 速度快,1GB 文本实测通常在 1~2 秒量级,满足"打开快"里"UI 不卡"这层要求),但峰值内存随文件大小线性增长(不像 on-demand 方案那样有上限),且不能像 on-demand 那样对"文档本身就有一段巨大到解析都嫌贵"的病态输入生效。这个回退方案技术上更简单、风险更低,若 spike 证实 on-demand 方案不可行或投入产出比不划算,直接切这条路。

### Tree 渲染:canvas 虚拟化,不跟随现有三个树的惯例

现有 Files / Database schema / Todo 分类三个树(`extensions/files/`、`extensions/database/view.rs`、`extensions/todo/`)都是"展开状态 DFS 拍平成 `Vec<Row>` + `column!`/`Scrollable`"的写法,没有一个做虚拟化——因为这三个场景从没遇到过"展开后可见行数大到需要虚拟化"的规模。JSON Tree 明确要支持大文件,沿用这套惯例会重新引入 tabular 修复之前的问题(可见节点一多就卡)。

因此改用 canvas 自绘,思路与 `tabular/grid.rs` 一致:每帧只对"当前展开路径拍平出的可见行"里落在 viewport 内的那一段做 draw。与 tabular 的差异是"总行数"不是固定的(取决于用户展开了多少),但虚拟化逻辑本身(可视窗口起点 + 逐行绘制)是同一套。

代价:不能直接复用 Files/Database/Todo 树的 `column!`/`Scrollable` 代码,是一份新写的 widget;能复用的只有 `byteui::interaction::icons`(`ChevronDown`/`ChevronRight`)与 `byteui::theme::icon_size::{chevron, tree_row_gap}` 这两个真正共享的图标/尺寸 token。

## 数据模型

新模块 `crates/dozer-app/src/json_tree/`。

```rust
// json_tree/mod.rs

/// 挂在 PreviewTab 上的运行时状态,与 tabular::TabularView 平行。
pub struct JsonTreeView {
    path: PathBuf,
    /// jsonl/ndjson: 每行一个根节点;单个 .json: 恒定一个根节点(下标 0)。
    pub roots: Vec<JsonNode>,
    /// 已展开的节点路径集合(NodePath -> 该节点当前是否展开)。
    /// 只存"展开"的,折叠是默认态,不用 O(全树) 的表。
    expanded: std::collections::HashSet<NodePath>,
    /// 正在等后台解码结果、还没等到 apply_node_loaded 回填的节点路径。
    /// 语义同 tabular::TabularView.loading_sheets(防重复 spawn)。
    loading_nodes: std::collections::HashSet<NodePath>,
    /// 当前视图模式:Tree 或原始文本(复用已加载的 CodeView,见"路由"一节)。
    pub view_mode: ViewMode,
    /// 可视窗口起点(拍平后的可见行下标),语义同 TabularView.scroll_row。
    pub scroll_row: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    RawText,
}

/// 树里一条路径:根到某节点经过的 key/下标序列,兼作 Tree 里的行身份、
/// expanded/loading 两个 HashSet 的键、以及懒加载时"重新定位到这个节点"
/// 的寻址依据(配合 ByteSpan 复用 sonic-rs 的偏移重入能力,若 spike 通过)。
pub type NodePath = std::rc::Rc<[PathSegment]>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// 单个 JSON 节点。`content` 为 None 表示"只知道形状,还没解码具体内容"
/// (对象/数组的子项列表、字符串/数字的具体值都算"内容"),点开时才后台
/// 解码填充,同 tabular 的 Sheet 懒加载。
pub struct JsonNode {
    pub kind: JsonKind,
    /// 字节范围,懒加载/回跳定位用(sonic-rs 偏移重入,若可行)。对单个
    /// `.json` 文件:是整个文件字节缓冲区里的偏移。对 `.jsonl`/`.ndjson`:
    /// 是**该行自己**字节缓冲区里的偏移(每行独立解析,偏移不跨行,根节点
    /// 的 `byte_range` 就是整行),不是相对整个文件的偏移——避免行边界
    /// 计算错位。
    byte_range: std::ops::Range<usize>,
    pub content: Option<NodeContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

pub enum NodeContent {
    /// object:key 顺序 + 子节点(子节点同样可能是"只知道形状"的懒占位)。
    /// 封顶 MAX_JSON_CHILDREN 个,超出打 truncated 标记(见"封顶策略")。
    Object { entries: Vec<(String, JsonNode)>, truncated: bool },
    Array { items: Vec<JsonNode>, truncated: bool },
    /// 叶子值已经是"展示字符串"形态,超长按 MAX_LEAF_PREVIEW_CHARS 截断。
    Leaf { display: String, truncated: bool },
}
```

## 封顶策略(近似统计,不追求精确)

沿用 tabular 那次修复确立的取舍——**近似统计,不为了一个精确数字去扫完整个文件/子树**:

| 维度 | 常量 | 超出时的行为 |
|---|---|---|
| 单个 object/array 展示的子项数 | `MAX_JSON_CHILDREN = 10_000` | 该节点 `truncated = true`,子项列表末尾追加一行"…还有更多,未精确统计总数"占位行,不给"共 N 项"的精确总数(算精确数等于扫完这个子树,违背"够数即停")。 |
| 单个叶子值(字符串/数字原文)展示长度 | `MAX_LEAF_PREVIEW_CHARS = 2_000` | 截断显示,末尾加省略号提示;不影响原始文本视图(那边显示未截断的原文,读的是文件原始字节,不经过这层封顶)。 |
| jsonl/ndjson 读取行数 | `MAX_JSON_LINES = 100_000`(同 `tabular::MAX_TABULAR_ROWS`) | 读满即停(同 CSV 早停,不继续扫到 EOF 只为计数),顶部提示条"仅显示前 10 万行(文件还有更多)"。 |
| 展开时的形状扫描深度 | 不做深度封顶,按需展开到子节点即止(树天生逐层展开,深度封顶意义不大——用户不点开就不会触发更深一层的扫描)。 | — |

## 加载与懒加载扩展

### 首次打开

`json_tree::load(path)` 在后台线程(`spawn_blocking`,同 tabular)跑:

- `.json`:用 sonic-rs 打开文档,只扫描根节点的**形状**(kind + 若为 object/array,子项 key/下标列表,不深入解码每个子项的值)——这是这次"打开快"的核心:哪怕文件是 1GB,只要根节点本身不是一个字段名多到离谱的单一 object,形状扫描的代价只跟根节点直接子项数相关,不跟文件总大小相关(子孙节点的解码全部推迟到用户点开那一层)。若 spike 证伪 on-demand 方案(见"关键技术决策"),这一步退化为"sonic-rs 全量 DOM 解析,只是渲染时才懒建 widget"。
- `.jsonl`/`.ndjson`:按行扫描字节偏移(不逐行完整解析成 Value,只需要知道"第 i 行在文件的哪个字节区间"),读满 `MAX_JSON_LINES` 即停;每行的具体内容(它是什么形状)在用户展开该行对应的根节点时才解码——原理与"单个 json 文件的根节点"完全一样,只是根节点从 1 个变成了 N 个(每行一个)。

### 展开触发的懒加载

用户点某个折叠节点的 chevron:

```rust
pub enum Action {
    Scroll { dy: i32 },
    ToggleExpand(NodePath),
    ToggleViewMode,
}

impl JsonTreeView {
    /// 纯状态转换(可单测,不做 IO)。展开一个"内容还没解码"的节点时返回
    /// NodeExpandRequest,交给外层(有 handle/proxy 的那一层,同 tabular 的
    /// SheetLoadRequest 套路)去后台解码。
    pub fn apply(&mut self, action: Action) -> Option<NodeExpandRequest> { .. }

    /// 后台解码完成后回填,同 apply_sheet_loaded。
    pub fn apply_node_loaded(&mut self, path: NodePath, result: Result<NodeContent, String>) { .. }
}

pub struct NodeExpandRequest {
    pub path: NodePath,
    pub byte_range: std::ops::Range<usize>,
}
```

`loading_nodes` 防重复 spawn 的逻辑与 tabular 修复后的 `loading_sheets` 完全一致(2026-09 code review 那次教训直接照搬,不重犯):同一节点在收到回填之前再次点开不重复触发。

## JSONL 处理细节

每一行独立成一个 `JsonNode`(`roots[i]`),折叠态显示"第 i 行:{Object, 大约 N 个字段}"这类形状摘要,不需要先解析出具体内容——形状信息(kind + 直接子项数)在初次扫描字节偏移时可以顺带做一次轻量探测(sonic-rs 的形状查询不需要完整解码)。

## 路由(委托)

`crates/dozer-app/src/preview/native_editor.rs` / `view.rs` 新增判定,风格同 tabular 的 `is_tabular_extension`:

```rust
pub fn is_json_tree_extension(path: &Path) -> bool {
    matches!(ext.as_str(), "json" | "jsonl" | "ndjson")
}
```

与 tabular 的关键差异:**不从 `is_editable_extension` 里摘除 `.json`**——`push_tab` 对这三个扩展名同时构造 `editor`(原生 CodeView,已有的 JSON 语法高亮路径,复用现状代码不动)与 `json_tree`(懒加载,`JsonTreeState::Loading` 起步,同 tabular 的 `TabularState`),两者都挂在 `PreviewTab` 上,`view_mode` 决定当前渲染哪一个。默认 `ViewMode::Tree`。

```rust
pub enum JsonTreeState {
    Loading,
    Ready(json_tree::JsonTreeView),
}
```

`PreviewTab` 新增字段 `json_tree: Option<JsonTreeState>`。注意这里**不是**"四选一互斥"——tabular 文件严格 `editor`/`tabular` 二选一(原设计不变);JSON 文件是唯一的例外,`editor` 与 `json_tree` **同时**非空(上面"路由"一节说的双视图前提),`view_mode` 只决定当前渲染哪一个,不代表另一个不存在。跟 tabular/webview 之间仍然互斥(一个文件不会同时是表格 tab 又是 JSON tab)。

派生影响,照抄 tabular 那次的检查单,逐条核对是否需要改:

- **webview 池判定**(`desired_webviews`/`active_webview_id`/`select` 的 `is_webview_file`):从 `editor.is_none() && tabular.is_none()` 再加一项 `&& json_tree.is_none()`——JSON tab 绝不能进 webview 池。
- **原生渲染判定**(`active_tab_is_native()`):**不需要改**。JSON tab 的 `editor` 本来就非空(双视图前提),现有的 `editor.is_some() || tabular.is_some()` 已经能正确识别它,这是双视图设计相对 tabular 的一个免费副作用。
- **Find/⌘F 全文搜索**:**不需要特殊处理,顺带可用**。`CodeView` 的 Find 面板只认 `editor.is_some()`,JSON tab 天然满足,用户切到原始文本视图就能直接用现成的 Find 搜索 JSON 原文定位——这是双视图设计比 tabular 多出来的能力,不是本计划要额外实现的目标,但值得在实现时确认没有被现有的 tabular 分支逻辑意外拦掉。
- **⌘S/撤销重做**:JSON 预览是只读(非目标里已声明不提供编辑入口),即使 `editor.is_some()`,也要确认 `CodeView` 走的是只读构造(同"未决问题"里提到的那条,需要在实现阶段确认,不能假设"有 editor 就等于可编辑保存")。
- **主题切换重载**(`reload_all_webviews_for_theme`):JSON tab 的 `editor` 非空但不该被这个函数当成"该重载的 wry tab"处理(它只推进 wry tab)——需要跟 tabular 当初那次一样显式排掉(该函数原本按 `editor.is_none()` 挑 wry tab,JSON tab 的 `editor` 非空能天然被排除,大概率不需要改,但要在实现时用真实 JSON tab 走一遍主题切换验证)。
- **`wry_toggle_eligible`**(预览/代码切换按钮判据):JSON 文件本来就是 `is_editable_extension` 覆盖范围(未摘除),需要确认加了 `json_tree` 路由之后这个既有的"预览/代码"切换按钮语义有没有被新加的"Tree/原始文本"切换按钮弄混——两者是不同维度的开关(一个是"渲染预览 vs 代码编辑器",一个是"树 vs 原始文本"),需要在实现阶段梳理清楚 UI 上会不会同时出现两个容易混淆的切换按钮,必要时这条在计划里单独定。

## UI 设计

### Tree(`json_tree/tree.rs`,canvas 虚拟化)

单节点一行,内容:

```
[缩进] [chevron(仅 object/array 且非空时显示)] [key 或下标] : [类型摘要或叶子值预览]
```

- object/array 折叠态:`{4 keys}` / `[128 items]`(或封顶时 `{10,000+ keys, 已截断}`)。
- 展开态:子项各自一行,缩进 +1 层。
- 叶子(string/number/bool/null):直接显示裁剪后的值,按类型着色(string 用 `body`,number/bool 用 `cream` 或既有强调色,null 用 `dim`)。
- chevron 复用 `byteui::interaction::icons`(`ChevronDown`/`ChevronRight`)+ `icon_size::chevron()`/`tree_row_gap()`,点击只切 `ToggleExpand`,不冒泡到行的其它交互(同 Files 树 `MouseArea` 独立命中的手法)。
- 只画可视窗口内的行(`scroll_row` 起点 + viewport 高度换算行数),与 `tabular/grid.rs` 同一套虚拟化算法,只是"总行数"换成"当前展开状态拍平出的可见行数"(每帧现拍平——只拍平已展开路径,不遍历整棵树,开销与"展开了多少"成正比,不与"文件多大"成正比)。

### 视图切换(`json_tree/view.rs`)

顶部一个二态切换按钮(Tree / 原始文本),复用 `byteui::interaction::icons::icon_button_entry`(CLAUDE.md 关键裁决:新增 icon 按钮优先复用这个,不手写 `MouseArea`)。切到原始文本时渲染已有的 `CodeView::view()`(只读态,`.json` 早已是 `is_editable_extension` 覆盖的语法高亮路径,原样复用,不需要新写)。

加载中(`JsonTreeState::Loading`)时不管 `view_mode` 是哪个,统一显示 `byteui::feedback::math_curve::loading_hint(Curve::RoseThree, "正在加载 JSON…", 48.0)`(同 tabular 的统一 loading 动画,不再包一层多余容器——直接吸取上次 code review 那条发现)。

## 消息与状态

沿用 tabular 已经跑通的整套异步模式,不重新发明:

```rust
// app/message.rs
JsonTreeAction(PanelKind, usize, json_tree::Action),
/// 首次打开的后台加载完成,project_id 路由(同 TabularLoaded)。
JsonTreeLoaded(ProjectId, PanelKind, usize, Result<json_tree::JsonTreeView, String>),
/// 某节点懒加载完成,回填进已存在的 JsonTreeView。
JsonNodeLoaded(ProjectId, PanelKind, usize, json_tree::NodePath, Result<json_tree::NodeContent, String>),
```

`Workspace::preview_pane_json_tree_action` / `spawn_pending_json_tree_loads` 直接照抄 `preview_pane_tabular_action` / `spawn_pending_tabular_loads` 的结构(`io.handle.spawn_blocking` + `io.proxy.send_event`),包括 `restore_preview_state` 里对已恢复的 JSON tab 并行 spawn 加载。

## 错误处理

| 场景 | 处理 |
|---|---|
| 文件不存在/无权限 | 同 tabular:`load` 返回 `Err`,该 tab 的 `editor`/`json_tree` 都为 `None`,走现有 webview/flyfish 兜底。 |
| 非法 JSON(语法错误) | `.json`:整个文件解析失败,`json_tree` 落 `Err`,tab 保留在 `Loading`(同 tabular 对 `TabularLoaded` 失败的取舍,不建 `Failed` 状态);原始文本视图仍然可用(纯文本读取不关心 JSON 是否合法),用户可以切过去看原文定位错误。`.jsonl`:单行解析失败只影响那一行(该根节点标 `Err`,显示"解析失败"叶子,其余行不受影响)——这是 jsonl 相对单文件 JSON 的一个真实优势,值得在 UI 上体现。 |
| sonic-rs 偏移重入失败(若该能力比预期更受限) | 见"关键技术决策"的回退方案,不在运行时兜底,是设计阶段就要敲定的路线。 |
| 展开一个节点时后台解码失败 | 同 `apply_sheet_loaded` 失败分支:该节点保持折叠占位 + 记日志,不在 UI 上展示内部错误码;`loading_nodes` 摘除,允许再次点开重试。 |

## 测试策略

- `PathSegment`/`NodePath` 相等性与 `HashSet` 键行为单测。
- `JsonTreeView::apply` 纯状态转换单测:展开触发 `NodeExpandRequest`、重复展开不重复 spawn(直接照抄 tabular 那条 `reselecting_still_loading_sheet_does_not_requeue_load` 回归测试的思路)、`apply_node_loaded` 成功/失败两支。
- 封顶逻辑单测:构造超过 `MAX_JSON_CHILDREN`/`MAX_LEAF_PREVIEW_CHARS`/`MAX_JSON_LINES` 的样本,断言 `truncated` 标记与实际展示内容符合预期,且不需要精确总数。
- `is_json_tree_extension` 覆盖三个扩展名 + 大小写 + 反例。
- jsonl 单行解析失败不影响其余行的端到端测试。
- sonic-rs on-demand 用法的 spike 本身产出一组基准/可行性测试(记录在 spike 任务里,不在这里重复列)。
- 大文件性能不做自动化断言(人工验证清单覆盖),但封顶路径全部有单测,同 tabular 先例。

## 依赖

```toml
# crates/dozer-app/Cargo.toml
sonic-rs = "0.3"  # 具体版本以 spike 时 crates.io 最新稳定版为准
```

若 spike 证伪 on-demand 路线、回退到"全量 DOM + 渲染时懒建 widget",依赖不变(`sonic_rs::Value` 是同一个 crate 的另一套 API),不需要换库。

## 未决问题

- `MAX_JSON_CHILDREN`/`MAX_LEAF_PREVIEW_CHARS` 的具体数值是这次给的建议值,实现阶段可能需要按真实大文件样本微调,不是硬性约束。
- 原始文本视图切换后,若用户在原始文本里手动修改了内容(理论上不该发生,因为这是只读预览,但要在实现阶段确认 `CodeView` 是否已经统一走只读态,不然会出现"树视图数据"和"文本视图内容"不一致的问题)。
