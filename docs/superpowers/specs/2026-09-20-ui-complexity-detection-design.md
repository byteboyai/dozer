# UI 复杂度检测设计

**状态：已批准（brainstorming 会话，2026-09-20，含技术可行性 spike）**

**关联 spec**：`docs/superpowers/specs/2026-09-20-code-health-panel-design.md`（v1，函数级结构复杂度）、`docs/superpowers/specs/2026-09-20-code-health-panel-ui-optimization-design.md`（v1 面板呈现层优化，已实现）。本文档是 v1 代码健康度体系的第二条腿——从"函数结构复杂度"扩展到"UI 一致性复杂度"，扩展同一个 `ProjectReport` 契约，同一个面板呈现。

## 背景

用户复盘代码健康度面板真实扫描数据后提出：能否再加一类检测，专门衡量 UI 代码本身的复杂度——色彩、边距、字体是不是散乱硬编码，组件化程度够不够。这类问题函数级圈复杂度看不出来（一个函数可以逻辑很简单、但颜色/间距写得到处都是魔法数字），是独立于"控制流复杂度"的另一个质量维度。

面向"通用能力，服务任何用户项目"来设计（同 v1 spec 的产品定位——本面板是甲方侧的验收层，不是只服务 Dozer 自己）。但不同 UI 框架（React/CSS、SwiftUI、iced、Android XML……）的"什么算硬编码/什么算复用"语法完全不同，同 v1"一期只支持 Rust"的裁决一样，**一期只做 iced**（复用 `dozer-codehealth` 已有的 ast-grep + Rust 解析基础设施），其余框架留作未来方向。

### 技术可行性 spike（会话中已完成）

`spike/ui-complexity/`（已提交，随时可删）用 `dozer-app`/`byteui` 两个真实 crate（205 个 `.rs` 文件，约 10 万行）验证了五项技术，其中两项有重要发现：

1. **性能坑（已找到并修复）**：`ast_grep_core::Node::find_all` 接受 `impl Matcher`，`&str` 也实现了 `Matcher`——但 `impl Matcher for str` 内部每次 `match_node` 调用都会重新 `Pattern::new(self, lang)` 编译一次 pattern，而 `find_all` 对树里**每个节点**都调用一次 `match_node`。直接传 `&str` 给 `find_all`，在单个 379 行文件上就要 6.6 秒（10 个 pattern × 全部节点）。**必须**用 `ast_grep_core::matcher::Pattern::new()` 在文件循环外把每个 pattern 预编译一次，循环内传 `&Pattern` 复用（`Pattern` 实现 `Matcher`，带 `potential_kinds()` 按节点种类快速跳过）。修复后全仓库扫描耗时从"文件循环里卡死"降到 **6.8 秒**（205 文件、约 10 万行）。这是本次唯一一个"必须做对，否则实现直接不可用"的技术风险，已用真实代码验证并解决。

2. **组件树嵌套深度：原方案（按 `macro_invocation` 节点种类数深度）技术不可行**。tree-sitter-rust **不会把嵌套在另一个宏参数里的宏调用识别成独立的 `macro_invocation` 节点**——`column![row![...], ...]`（iced 代码里随处可见的写法）里，外层 `column!` 能正常识别成 `macro_invocation`，但内层 `row!` 会退化成一串没有类型的裸 token（`identifier "row"` + `"!"` + `token_tree`），不管是按 `node.kind()` 过滤还是走 `find_all` pattern 匹配（`row![$$$]`）都找不到它——在 `codehealth/view.rs` 里已知存在的 `column![row![...]]` 嵌套上用两种方式实测，均返回 0 命中。这是 tree-sitter 对宏体不做深度语义解析的已知限制，不是实现 bug。**结论：嵌套深度指标改用基于源码文本的方括号扫描实现**（在函数体源码范围内找 `row![`/`column![` 字符串、手写一个只认这两个 token 的迷你状态机数深度），不依赖 AST 语义，精度是"文本启发式代理指标"，同其余几个字面量维度一样不追求精确值。

3. **颜色/边距/字体（字面量参数检测）：验证通过**。用 `$RECV.padding($X)`/`$RECV.spacing($X)` 等 pattern 配合 `MetaVarEnv::get_match()` 拿到捕获的 `$X` 节点，按 `kind()`（`integer_literal`/`float_literal`/`array_expression`/`string_literal`）判断是不是字面量。真实数据：`.padding(字面量)` 161 处、`.spacing(字面量)` 238 处、`Color::from_rgb*(...)` 构造调用 11 处、distinct 字面量值种类数 38。这套技术即使目标调用嵌套在另一个宏的 token_tree 内部也能命中（token_tree 内部的叶子 token——`identifier`/`integer_literal`/标点——仍然各自保留正确的 kind，只是不会被重新组合成 `call_expression`/`macro_invocation` 这类高层节点；ast-grep 的 pattern 匹配对这种"扁平但叶子有类型"的结构依然有效），这也是为什么嵌套深度失败但字面量检测不受影响的原因。

4. **事件回调密度：验证通过**，复用第 3 项同一套 `find_all` 技术（换个 pattern），真实数据：`on_press`/`on_enter`/`on_exit`/`on_input`/`on_submit` 总调用 143 处，无额外风险。

5. **组件化重复结构（结构指纹去重）：验证通过，但要认清精度边界**。把 `row!`/`column!` 顶层宏调用的子树序列化成"只保留节点种类排列、丢弃字面量文本"的字符串，同指纹分组。真实数据：8 个重复次数 ≥3 的指纹簇（最多 86 次）。**局限**：由第 2 项同样的 tree-sitter 限制导致，指纹只能可靠反映"顶层结构 + 内部退化成扁平 token 的序列"，不是教科书级、完全递归感知层级的 clone detection——但两个真正结构相同的 widget 树，内部退化的方式是一致的，所以指纹比对依然有效，只是不能用"这是标准 AST clone detection"这种更强的说法去描述它。

## 目标 / 非目标

**目标：**

1. 扩展 `crates/dozer-codehealth` 的 `ProjectReport`，新增 UI 一致性相关字段（见"指标定义"），复用现有的文件遍历 + `.rs` 收集流水线，不新建 crate。
2. 六个检测维度：颜色硬编码、边距硬编码、字体硬编码、组件树嵌套深度、事件回调密度、组件化重复结构。附带统计：调色板/间距值种类数（挂在颜色/边距两个维度下的补充数字，不是独立发现列表）。
3. `FunctionMetric` 新增两个字段（`widget_nesting_depth`、`event_handler_count`），随现有函数级扫描一起产出，不新建单独的遍历。
4. 面板问题列表下方新增"UI 一致性"区块，按维度**分类分区**展示（不是混进现有函数结构复杂度那个排行榜——发现的性质差异太大，混排反而看不懂，这条在 brainstorming 阶段已经问清楚）。
5. 严重度阈值全部写死常量，凭经验定，可调整（同 v1 一贯做法，不做每项目自适应归一化、不做配置面板）。

**非目标：**

- **不支持多框架**。一期只做 iced/Rust；React/CSS、SwiftUI 等留作未来方向（每个框架的"硬编码/复用"语法和检测规则都要重新设计，不是加个 if 分支的事）。
- **不做按路径排除清单**。颜色/字体 token 的定义文件本身会包含合法的原始字面量构造（比如 `ColorTokens` 结构体初始化、`code_font()` 里的 `Font::with_name(CODE_FONT_FAMILY)`），这些会被计入发现数——这是已知可接受的局限（同函数级阈值一样，如果发现系统性偏差可以直接调整常量），不在 v1 做"按项目路径手动配置排除列表"这种需要用户维护配置的方案，避免每个项目都要先来一轮排除配置才能用。
- **不做真正意义上的 clone detection**（如 AST 归一化 + 编辑距离/子树哈希的完整实现）。结构指纹去重是"够用的启发式代理"，见 spike 结论第 5 条。
- **不做嵌套深度的精确 AST 版本**。已在 spike 验证技术不可行（tree-sitter 限制），改用文本扫描代理指标，不再追加尝试其他 AST 方案。
- **不做每项目/每用户可配置阈值**。同 v1。

## 指标定义

### 颜色硬编码

匹配颜色字面量构造调用：`Color::from_rgb($$$)`/`Color::from_rgba($$$)`/`Color::from_rgb8($$$)`。**构造函数调用本身就是发现**（不需要像边距那样区分参数是不是字面量）——因为项目里对颜色 token 的正确用法是**字段访问**（如 `tokens.red`），不是再调一次构造函数；任何地方出现构造函数调用都是"没有走 token 系统"的信号。

```rust
pub struct RawLiteralFinding {
    pub file: PathBuf,
    pub line: usize,
    pub snippet: String, // 命中的源码文本,如 "Color::from_rgb(0.043, 0.055, 0.086)"
}
```

项目级严重度（发现总数）：`0` Healthy，`1..=15` Watch，`>15` Critical。真实数据（Dozer 自身）11 处，落在 Watch。

补充统计：`distinct_color_values: usize`——对捕获到的字面量文本去重计数，衡量"这个项目实际用了多少种不同颜色"，调色板越大说明设计系统越松散。不设独立严重度，随发现列表一起展示为一个数字。

### 边距硬编码

匹配 `$RECV.padding($X)`/`$RECV.spacing($X)`，且 `$X` 的 `kind()` 属于 `{integer_literal, float_literal, array_expression, unary_expression}`（负数字面量在 tree-sitter-rust 里是 `unary_expression`）——这次**要**区分字面量和变量引用：`.padding(spacing_token)` 不该被抓，`.padding(8)` 该被抓。

项目级严重度：`0..=10` Healthy，`11..=50` Watch，`>50` Critical。真实数据（Dozer 自身）padding 161 + spacing 238 = 399 处，落在 Critical——这是真实信号：这个仓库目前确实没有间距 token 系统（CLAUDE.md 关键裁决只覆盖了颜色 token，没有间距 token），不是检测误报。

补充统计：`distinct_spacing_values: usize`，同颜色的调色板统计，对 padding/spacing 两类字面量文本合并去重计数。

### 字体硬编码

匹配 `$RECV.font($X)`/`Font::with_name($X)`，且 `$X` 的 `kind()` 是 `string_literal`（**这次和颜色不同，要区分字面量和常量引用**——原因：这个仓库里字体的正确用法是通过 `code_font()` 这样的封装函数访问，封装函数内部只调用一次 `Font::with_name(CODE_FONT_FAMILY)`，`CODE_FONT_FAMILY` 是个具名常量（`identifier` 节点），不是裸字符串字面量；如果到处都是 `Font::with_name("JetBrains Mono")` 这种裸字符串重复出现，才是真正的"没走统一入口"信号）。

项目级严重度：`0` Healthy，`1..=5` Watch，`>5` Critical（字体场景天然应该只有个位数的封装点，阈值比颜色/边距更严格）。

### 组件树嵌套深度

**实现方式（spike 结论调整过）**：不用 AST 节点种类判断，改成对函数体的**源码文本**做方括号深度扫描——在函数体 `[start_line, end_line]` 对应的源码切片里，识别 `row![`/`column![` 这两个具体 token（用简单的字符串匹配 + 括号计数状态机，不需要完整词法分析器），每遇到一次视为深度 +1，遇到与之匹配的 `]` 视为深度 -1，取函数体内出现过的最大深度。

```rust
pub fn widget_nesting_depth(fn_source: &str) -> usize {
    // 手写迷你状态机:只认 "row![" / "column![" 开括号和对应的 "]" 闭括号,
    // 忽略其余所有字符(含字符串/注释里偶然出现的同名文本——已知局限,
    // 见"非目标")。
}
```

这个函数挂到 `FunctionMetric::widget_nesting_depth: usize`，随现有函数级扫描一起产出（不需要新的文件遍历）。

函数级严重度（独立于现有 `complexity_signal`/`severity`，不污染现有字段）：`depth <= 2` Healthy，`3..=4` Watch，`depth > 4` Critical。

### 事件回调密度

匹配 `$RECV.on_press($$$)`/`on_enter`/`on_exit`/`on_input`/`on_submit`，每个函数体内命中总数记为 `FunctionMetric::event_handler_count: usize`。

函数级严重度：`0..=3` Healthy，`4..=6` Watch，`>6` Critical。

### 组件化重复结构

对每个**顶层**（不嵌套在另一个宏参数内的）`row!`/`column!` 宏调用节点，计算"结构指纹"（子树 dfs，只拼接 `node.kind()`，丢弃字面量文本）。全项目按指纹分组，出现次数 ≥3 的组判定为一个"重复簇"：

```rust
pub struct DuplicateCluster {
    pub occurrences: Vec<(PathBuf, usize)>, // (文件, 起始行),按出现顺序
    pub node_count: usize, // 该结构指纹的规模(fingerprint 字符串长度的量级信号,用于排序"这堆重复有多大")
}
```

严重度（按簇内出现次数）：`3..=5` Watch，`>5` Critical（没有 Healthy 档——低于 3 次根本不构成"簇"，不会出现在列表里）。真实数据：8 个簇，最大 86 次（这是 `app/update.rs` 里一个巨大 match 语句的信号，和 v1 spec 里已经发现的"`update` 函数是全仓库结构最严重的函数"是同一处问题的另一个侧面）。

## 架构与数据流

### 1. `ProjectReport` 扩展

```rust
pub struct ProjectReport {
    // === 现有字段不变 ===
    pub total_loc: usize,
    pub total_functions: usize,
    pub critical_functions: usize,
    pub scale_tier: HealthTier,
    pub density_tier: HealthTier,
    pub overall_tier: HealthTier,
    pub functions: Vec<FunctionMetric>, // 新增两个字段,见下

    // === 新增 ===
    pub color_findings: Vec<RawLiteralFinding>,
    pub spacing_findings: Vec<RawLiteralFinding>,
    pub font_findings: Vec<RawLiteralFinding>,
    pub distinct_color_values: usize,
    pub distinct_spacing_values: usize,
    pub duplicate_clusters: Vec<DuplicateCluster>,
    pub ui_tier: HealthTier, // 见下方"UI 一致性分档"
}

pub struct FunctionMetric {
    // === 现有字段不变 ===
    pub name: String,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub loc: usize,
    pub complexity_signal: usize,
    pub severity: Severity,

    // === 新增 ===
    pub widget_nesting_depth: usize,
    pub event_handler_count: usize,
}
```

`overall_tier` 的计算规则**不变**（`max(scale_tier, density_tier)`，仍然只由函数结构复杂度和代码规模决定）——`ui_tier` 是一个平行的新维度，**不**并入 `overall_tier` 的 max 计算,原因：健康卡片已经用"规模/密度"两行说明 `overall_tier` 的成因（见 UI 优化 spec），UI 一致性是完全不同性质的问题（"代码逻辑好不好"vs"UI 写法规不规范"），混进同一个总分会让"综合等级"的可解释性变差，不符合 v1 spec 反复强调的"可解释性优先"原则。`ui_tier` 单独展示在"UI 一致性"区块自己的标题上。

`ui_tier` 计算规则：取以下六个子分档里最严重的一档，同现有 `overall_tier` 的"取最严重"哲学一致：

1. 颜色发现数对应的项目级严重度
2. 边距发现数对应的项目级严重度
3. 字体发现数对应的项目级严重度
4. 全部函数里 `widget_nesting_depth` 最严重那个的严重度
5. 全部函数里 `event_handler_count` 最严重那个的严重度
6. 全部重复簇里出现次数最多那个的严重度（没有簇则视为 Healthy）

### 2. 扫描流程扩展

在 `crates/dozer-codehealth/src/report.rs::scan_project` 现有的"逐文件读取 → 解析 → 聚合"循环里，每个文件解析出 `functions_in_source` 之后，追加：

1. 对该文件的 AST 根节点跑颜色/边距/字体三组预编译 pattern（`Patterns` 结构体，文件循环外只 `Pattern::new()` 一次，见"已知风险"）。
2. 对每个函数，用其 `[start_line, end_line]` 对应的源码切片跑 `widget_nesting_depth` 文本扫描 + 事件回调 pattern 计数，填进 `FunctionMetric` 新字段。
3. 收集该文件的顶层 `row!`/`column!` 节点，计算结构指纹，汇总进全局 `HashMap<String, Vec<(PathBuf, usize)>>`（同 spike 实现）。

全部完成后，在文件循环外：过滤指纹计数 ≥3 的组成 `duplicate_clusters`；对字面量发现文本去重得到两个 `distinct_*_values`；计算 `ui_tier`。

### 3. `dozer-app` 面板呈现

`crates/dozer-app/src/extensions/codehealth/view.rs` 的 `content_pane` 在现有"健康卡片 + 问题列表"下方新增一个"UI 一致性"区块，**分类分区**展示：

- 颜色（发现列表 + 调色板种类数）
- 边距（发现列表 + 间距值种类数）
- 字体（发现列表）
- 组件树嵌套深度（超标函数列表，按 `widget_nesting_depth` 降序）
- 事件回调密度（超标函数列表，按 `event_handler_count` 降序）
- 组件化重复结构（重复簇列表，每簇展示出现次数 + 全部位置，点击跳转复用现有 `Message::OpenLocation`）

每个子区块标题旁标严重度徽章（复用现有 `tier_color`/`tier_label`），配色和现有面板同一套 `ColorTokens`，不新增 token；字体沿用默认（不显式设置 `.font(...)`）；不使用 `gold`。这几条延续 UI 优化 spec 里已经核实过的"UI token/组件一致性"结论，本文档不重复验证。

## 已知局限

1. **组件树嵌套深度是文本启发式，不是语义精确值**——字符串/注释里偶然出现的 `"row!["`/`"column!["` 文本会被误判为一次嵌套。概率很低（这种字面量文本在真实代码里几乎不会出现），且和函数级 `complexity_signal` 本来就是"McCabe 复杂度的粗代理，不追求精确"的定位一致，可接受。
2. **组件化重复结构的指纹精度受 tree-sitter 宏体不解析的限制**（见 spike 结论第 5 条），是"顶层结构 + 内部退化成扁平 token 序列"的混合信号，不是标准 clone detection。
3. **颜色/字体 token 定义文件本身会被计入发现数**（没有做路径排除），首次扫描 Dozer 自己的仓库时，`byteui::theme::color::ColorTokens` 的定义文件、`assets/fonts.rs` 会因为合法初始化代码而贡献一部分发现数。这是已知可接受的局限（同"非目标"一节），不做特殊处理。
4. **边距硬编码的阈值按 Dozer 自身真实数据（399 处）校准**，如果放到一个已经有完善间距 token 系统的项目上跑，Healthy 档的上限（10）可能显得过于严格——这是"写死常量、凭经验定、可调"的已知局限，不是 bug。

## 测试策略

- `dozer-codehealth`：
  - 颜色/边距/字体三组 pattern 的字面量 vs 变量引用判定边界（构造已知字面量/已知变量两种 fixture）。
  - `widget_nesting_depth` 文本扫描：已知深度 1/2/3 的嵌套 `row!`/`column!` fixture，边界值（`depth<=2`/`3..=4`/`>4`）。
  - `event_handler_count`：已知回调数量的 fixture 函数，边界值（`0..=3`/`4..=6`/`>6`）。
  - `structural_fingerprint` + 分组：构造两个结构相同、字面量不同的 `row!` fixture，验证指纹相同；构造两个结构不同的，验证指纹不同。
  - `ui_tier` 取三个维度较严重者的 max 逻辑（同 `overall_tier` 现有测试模式）。
  - **性能回归测试**：用一个较大的 fixture 目录（或直接在测试里断言"预编译 pattern 复用，不在循环内调用 `Pattern::new`"这类实现约束，具体形式留给 plan 阶段核实——重点是不能让 spike 发现的性能坑在实现里复现）。
- `dozer-app::extensions::codehealth`：面板新区块渲染留给 `cargo run -p dozer-app` 人工验收，同现有面板惯例。

## 依赖变更

无新增依赖——复用 `dozer-codehealth` 已有的 `ast-grep-core`/`ast-grep-language`。

## 未来方向

以下方向在本次 brainstorming 中讨论过，**均明确排除在本次范围外**：

1. **多框架支持**：React/CSS、SwiftUI、Android XML 等，每个框架需要重新设计"什么算硬编码/什么算复用"的检测规则（同 v1 spec"多语言支持"未来方向同等级别的工作量）。
2. **组件树嵌套深度的语义精确版本**：如果未来 tree-sitter-rust 或 ast-grep 支持宏体的深度语义解析（或者切换到编译器级别的工具如 `rustc` 的 `proc_macro`/HIR），可以把文本启发式换成真正的 AST 版本。
3. **按路径排除清单**：让用户手动配置"这些文件是 token 定义文件，不计入发现数"，用来消掉"已知局限"第 3 条的噪音。需要配置 UI + 持久化，同 v1 spec"阈值可配置"一样的工作量级别，不在 v1 做。
4. **真正意义上的 clone detection**：AST 归一化 + 子树哈希/编辑距离的完整实现，比当前的"结构指纹"更精确，但需要解决 tree-sitter 宏体不解析的根本限制才有意义。
