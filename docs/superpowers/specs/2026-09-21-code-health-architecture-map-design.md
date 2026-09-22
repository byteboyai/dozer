# 代码健康度下一迭代设计：架构地图与依赖风险

**状态：草案（2026-09-21）**

**关联设计：**

- `2026-09-21-code-health-evolution-design.md`：可信扫描、历史差异、Git 热点与右侧分类导航。
- `2026-09-20-code-health-panel-design.md`：代码健康度一期数据层和手动扫描。

## 背景

当前代码健康度能够回答“哪些函数复杂”“本轮新增了什么风险”，但不能回答代码量增长后更关键的架构问题：

1. 项目由哪些 workspace crate 和内部模块组成；
2. 模块之间如何依赖，是否出现环；
3. 哪些模块承担过多职责或成为高耦合枢纽；
4. 本轮修改扩大了哪些模块的影响范围；
5. 代码是否穿透了项目声明的架构边界。

Vibe coding 中的失控通常不是单个函数突然变长，而是新功能不断连接已有模块、绕过边界，并让少数文件成为所有功能的共同依赖。下一迭代将这些关系变成可浏览、可比较和可交给 Agent 分析的架构地图。

## 产品原则

1. **先给可读的项目地图，再呈现风险。** 首次进入架构页也必须帮助用户理解项目，不要求项目已经存在问题。
2. **只展示能够从源码证明的关系。** 边必须能回溯到 `Cargo.toml` 或具体 `use` 位置；不把名称相似推断成依赖。
3. **默认控制信息量。** 首屏显示 workspace crate 和一级模块，用户按需展开，不一次绘制全部文件和函数。
4. **风险必须可解释。** 不提供不可解释的架构总分；展示环、扇入/扇出、边界违规、变更和影响范围。
5. **历史变化优先。** 新增依赖、新增环和影响范围扩大优先于长期存在的结构问题。

## 目标

1. 从 Cargo workspace 和 Rust 源码构建可持久化的 crate/module 依赖图。
2. 在代码健康面板增加“架构”分类，提供可缩放、可平移、可展开的交互式架构图。
3. 检测循环依赖、依赖枢纽和项目配置声明的分层边界违规。
4. 比较相邻扫描的节点和边，展示新增、移除和持续存在的架构关系。
5. 结合 Git 改动文件计算本轮直接影响范围，并从图中定位风险来源。
6. 支持从节点、边和架构发现跳转源码，或生成只读诊断上下文交给 Agent。

## 非目标

- 不生成函数调用图。Rust 动态分派、宏展开和条件编译使可靠调用图需要编译器级分析，本迭代只分析 crate 与 module 关系。
- 不调用外部 `cargo-modules`、Graphviz 或 Mermaid 作为运行时依赖。它们可作为行为参考；Dozer 的扫描和 UI 必须在未安装额外命令时工作。
- 不把架构图当作自动生成的正式系统设计文档；图描述的是静态代码关系。
- 不自动修改模块、移动文件或解除依赖。
- 不扫描第三方 crate 的内部模块；第三方依赖只在 crate 层作为可选节点展示。
- 不在本迭代支持 Rust 以外语言的语义依赖图。

## 用户流程

### 查看项目结构

1. 用户完成一次代码健康扫描。
2. 进入右侧“架构”分类。
3. 首屏显示 workspace crate、一级模块、它们之间的依赖和架构风险摘要。
4. 用户点击节点查看职责线索、代码量、依赖方向、近期变更和健康发现。
5. 用户展开节点查看下一级模块；双击或使用“打开源码”跳转对应文件。

### 检查本轮架构变化

1. 用户重新扫描。
2. 架构页默认突出显示本轮新增边、新增环和影响范围扩大的节点。
3. 用户选择一条边，查看该关系来自哪些 `use` 或 Cargo 依赖声明。
4. 用户将发现交给 Agent 分析，诊断上下文包含关系两端、证据位置、变化和验收目标。

### 声明项目边界

项目可在 `.dozer/code-health.toml` 增加可选配置：

```toml
[architecture]
max_fan_out = 8
critical_fan_out = 15
impact_depth = 3

[[architecture.layers]]
name = "ui"
match = ["crates/dozer-app::app/**", "crates/dozer-app::extensions/**"]
may_depend_on = ["application", "shared"]

[[architecture.layers]]
name = "application"
match = ["crates/dozer-core/**"]
may_depend_on = ["domain", "shared"]

[[architecture.layers]]
name = "domain"
match = ["crates/dozer-domain/**"]
may_depend_on = ["shared"]
```

没有配置时仍生成架构图并检测循环与枢纽，但不猜测分层规则，也不产生边界违规。

## 信息架构与 UI

### 分类导航

右侧分类导航从四项扩展为五项：

1. 总览
2. 结构复杂度
3. 架构
4. UI 一致性
5. 扫描范围

“架构”行显示当前风险数量；存在本轮新增循环或边界违规时使用警戒色，同时保留文字数量。

### 架构页布局

左侧详情区从上到下包含：

1. **页头：** “架构地图”、扫描基准、图层选择（crate / module）、“仅看风险”开关和重置视图。
2. **摘要：** 节点数、关系数、循环数、边界违规数、本轮新增关系数。
3. **图画布：** 支持滚轮缩放、拖拽平移、节点选择、模块展开/折叠和适配窗口。
4. **选中项详情：** 节点或边的指标、依赖列表、源码证据、近期修改次数和相对上次变化。
5. **风险列表：** 新增循环、边界违规和依赖枢纽；点击后选择并居中图中对象。

### 图形语义

- workspace crate：较大的圆角矩形；module：普通圆角矩形。
- 实线边：本项目内部依赖；虚线边：第三方 crate 依赖。
- 箭头 `A → B` 表示 A 依赖 B。
- 新增边：青色并带“新增”文字；边界违规：红色；循环中的边：黄色。
- 节点颜色表达当前风险，同时显示“循环”“边界违规”“高扇出”等文字徽标。
- 颜色不能作为唯一信息来源。
- 默认隐藏无风险第三方节点；用户可显式开启。

### 大图降噪

- 首屏最多绘制 80 个可见节点和 160 条边。
- 超过限制时按 crate/父 module 聚合，并显示“还有 N 个模块”；用户展开后替换其他分支，不无限追加。
- 相同起点与终点的多个 `use` 聚合为一条边，详情中列出全部证据。
- 画布只绘制当前可见子图；完整图仍用于风险分析。

### 空状态和降级

- 尚未扫描：沿用代码健康统一空状态并提供“开始扫描”。
- 非 Cargo Rust 项目：显示 module 图，并标注“未发现 Cargo workspace”。
- 没有 Rust 语义分析：显示“当前项目暂无可生成架构图的 Rust 代码”，不可显示为健康。
- Cargo metadata 失败：module 图继续可用，在扫描范围页记录失败原因。
- 图过大：显示聚合图和截断说明，不让 Canvas 卡死。

## 数据模型

报告 schema 从 `2` 升级到 `3`。旧报告缺少 `architecture` 时使用默认空值；UI 提示重新扫描，不把空图解释为零风险。

```rust
pub struct ProjectReport {
    pub schema_version: u32,
    #[serde(default)]
    pub architecture: ArchitectureReport,
    // 现有字段保持兼容。
}

pub struct ArchitectureReport {
    pub status: ArchitectureStatus,
    pub nodes: Vec<ArchitectureNode>,
    pub edges: Vec<ArchitectureEdge>,
    pub cycles: Vec<DependencyCycle>,
    pub unresolved_edges: usize,
}

pub enum ArchitectureStatus {
    Complete,
    Partial,
    NotApplicable,
}

pub struct ArchitectureNode {
    pub id: String,
    pub kind: ArchitectureNodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: Option<PathBuf>,
    pub parent_id: Option<String>,
    pub loc: usize,
    pub fan_in: usize,
    pub fan_out: usize,
    pub layer: Option<String>,
    pub external: bool,
}

pub enum ArchitectureNodeKind {
    Workspace,
    Crate,
    Module,
    ExternalCrate,
}

pub struct ArchitectureEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: ArchitectureEdgeKind,
    pub evidence: Vec<ArchitectureEvidence>,
}

pub enum ArchitectureEdgeKind {
    CargoDependency,
    ModuleUse,
}

pub struct ArchitectureEvidence {
    pub path: PathBuf,
    pub line: usize,
    pub snippet: String,
}

pub struct DependencyCycle {
    pub id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
}
```

### 稳定标识

- crate 节点 ID：`crate:<Cargo package name>`；同名 workspace package 再加入规范化 manifest 相对路径。
- module 节点 ID：`module:<crate name>::<Rust module path>`。
- external crate 节点 ID：`external:<package name>`。
- 边 ID：`edge:<kind>:<from>:<to>`，不包含行号和证据数量。
- cycle ID：对环内节点 ID 排序后计算稳定哈希，不依赖 DFS 返回顺序。

稳定 ID 用于历史比较。源码移动导致 qualified name 改变时，在本迭代被视为删除旧节点并新增新节点，不做模糊重命名推断。

## 关系提取

### Cargo workspace 图

使用 `cargo_metadata`，以项目根目录的 `Cargo.toml` 为入口：

- 只把 workspace member 建为内部 crate 节点；
- workspace member 之间的依赖建 `CargoDependency` 边；
- 第三方直接依赖可建 external 节点，但默认不展示；
- 使用 `--no-deps` 等价能力，避免下载依赖或扩大扫描范围；
- target-specific 和 optional 依赖保留在边证据/属性中，首版 UI 聚合展示。

### module 图

复用每个 Rust 文件已经解析的 AST：

- 根据 crate root、文件路径、`mod foo;` 和内联 `mod foo {}` 建 module 层级；
- 解析 `use crate::...`、`use self::...`、`use super::...` 和 workspace crate 前缀；
- 将具体 item 路径归并到最深可解析 module；
- `pub use` 仍表示依赖，同时在证据中记录 re-export；
- 无法可靠解析的 use 计入 `unresolved_edges`，不创建猜测边；
- `cfg` 导致的多种可能关系按静态出现记录，并在证据中标记条件属性。

扫描器必须让结构指标、UI 规则和架构提取共享同一次 AST 解析。

## 图分析与健康规则

内部实现使用 `petgraph` 或等价的纯 Rust 图算法。持久化仍使用上述稳定、与图库无关的数据结构。

### 循环依赖

- 对内部 crate 图和 module 图分别计算强连通分量。
- 两个及以上节点的 SCC，或节点到自身的自环，产生循环发现。
- 同一个 SCC 只产生一条发现，证据列出构成闭环的节点和代表性边。
- 新出现的 cycle ID 进入“本轮新增”；消失的进入“已解决”。

### 依赖枢纽

- `fan_out >= max_fan_out`：Watch；`fan_out >= critical_fan_out`：Critical。
- 默认阈值为 8 和 15，可由项目配置覆盖。
- `fan_in` 只作为影响范围信号，不单独判错；被大量模块复用的稳定基础模块可能是合理设计。
- 枢纽发现证据必须显示具体扇出值和最主要目标。

### 分层边界

- 节点先按 `architecture.layers[].match` 匹配到 layer；按配置顺序首个匹配生效。
- 已匹配节点的出边目标 layer 不在 `may_depend_on` 中时，产生 Critical 发现。
- 未匹配节点显示“未分层”，但不产生违规。
- 配置中的未知 layer、重叠规则和无效 glob 在扫描范围中记录配置错误，使架构状态为 `Partial`。

### 影响范围

- 以 Git dirty 文件和相对上次快照发生变化的 module 为起点。
- 沿反向依赖边计算最多 `impact_depth` 层的受影响内部节点，默认深度 3。
- UI 分开展示直接依赖方和间接影响范围，不把它们计为静态缺陷。
- 本轮影响节点数增加时显示变化，但不单独提升整体健康档位。

## 统一发现与差异

`FindingCategory` 增加 `Architecture`，`FindingEvidence` 增加：

```rust
ArchitectureCycle { node_ids: Vec<String> }
ArchitectureHub { node_id: String, fan_out: usize }
ArchitectureBoundary {
    edge_id: String,
    from_layer: String,
    to_layer: String,
}
```

规则 ID：

- `architecture/dependency_cycle`
- `architecture/high_fan_out`
- `architecture/layer_violation`

边和节点差异独立于 Finding 差异：

```rust
pub struct ArchitectureDiff {
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
    pub added_cycles: Vec<String>,
    pub resolved_cycles: Vec<String>,
}
```

首次扫描只显示“首次扫描，无架构变化基线”，不把所有现有边标为用户可见的“新增风险”。底层可以计算 added 集合，但 ViewModel 必须区分无基线。

## 布局与渲染

### 布局

- crate 层使用从左到右的分层有向图布局。
- module 层按父节点分组；同层节点按稳定 ID 排序，保证重复扫描后位置稳定。
- 环中的节点放在相邻层并用回边连接，不要求消除所有交叉线。
- 布局计算是纯函数，输入可见节点和边，输出节点矩形、边折线和整体边界。
- 相同输入必须得到相同坐标，方便测试和避免 UI 抖动。

本迭代不引入 Graphviz 进程。若简单分层布局在真实项目中无法满足可读性，再单独评估纯 Rust 布局库。

### iced Canvas

新增独立 Canvas 模块，负责：

- 绘制节点、端口、边、箭头和状态徽标；
- 命中测试；
- 鼠标滚轮缩放，缩放范围 0.5–2.5；
- 按住空白区域拖拽平移；
- 单击选择，双击模块展开/折叠；
- “适配窗口”根据布局边界重置 viewport。

Canvas 不执行业务分析，不持有唯一业务状态。当前选择、展开集合、过滤器和 viewport 由代码健康 WorkspaceState 或明确的 Canvas local state 管理。

## Agent 诊断上下文

架构发现的诊断文本包含：

- 规则与严重度；
- 涉及节点和依赖方向；
- 形成关系的源码证据；
- 是否为本轮新增；
- Git 修改频率和影响范围；
- 配置中的预期边界；
- 验收目标。

示例验收目标：

- “`ui → infrastructure` 违规边消失，且没有新增同类边界违规。”
- “该 SCC 不再包含两个以上节点，重新扫描后循环发现消失。”
- “模块扇出从 17 降至配置阈值 8 以下，且公共行为测试通过。”

仍沿用现有行为：只把文本写入 Agent 输入区，不自动发送或修改代码。

## 性能与资源约束

- 架构提取复用已有文件遍历和 AST，不允许为 module 图再次读取或解析全部 Rust 文件。
- 10 万行项目的架构分析额外耗时目标低于 3 秒；整体扫描仍保持 15 秒目标。
- 完整报告默认最多保存 5,000 个内部 module 节点和 20,000 条聚合边；超过时状态为 `Partial` 并记录截断原因。
- SCC、扇入/扇出和影响范围计算目标为 O(V + E)。
- Canvas 每帧只处理可见子图，不遍历完整图做命中测试。

## 错误处理

- Cargo metadata 失败：保留 module 分析，架构状态为 `Partial`。
- 单个 Rust 文件解析失败：沿用扫描范围跳过记录，架构状态为 `Partial`。
- use 无法解析：增加 `unresolved_edges`，不使整次扫描失败。
- 架构配置无效：忽略无效规则，展示可定位错误，状态为 `Partial`。
- 图达到上限：保存已构建部分和截断原因；UI 明确显示分析不完整。
- 旧快照没有架构字段：不计算架构差异，提示重新扫描建立基线。

## 测试与验收

### 自动测试

- 单 crate、多 crate workspace、path dependency 和第三方依赖提取。
- 文件 module、内联 module、`crate/self/super` use、别名和聚合 use。
- 无法解析路径只增加 unresolved 计数，不产生假边。
- SCC、自环、无环图、稳定 cycle ID。
- 扇入/扇出阈值与项目覆盖配置。
- layer glob 匹配、允许边、违规边、未知 layer 和重叠规则。
- 节点、边和循环的相邻快照差异。
- schema v2 报告反序列化成空架构数据并提示需要重扫。
- 布局确定性、矩形不重叠、折叠后节点数量和最大可见数量。
- Canvas 命中测试、缩放边界、平移和重置视图的纯逻辑部分。
- 架构发现生成的 Agent 文本包含证据和验收目标。

### 人工验收

1. 扫描 Dozer 仓库，架构页首屏显示 workspace crate 和稳定的依赖方向。
2. 切换 module 图后可展开 `dozer-app::extensions`，再次扫描后布局不随机跳动。
3. 点击节点显示 LOC、扇入、扇出、依赖列表和近期变动；打开源码能定位对应文件。
4. 在 fixture 中制造 A → B → A，重新扫描后显示新增循环，图与列表互相定位。
5. 配置禁止 `domain → ui`，制造违规后显示配置预期和实际证据位置。
6. 删除循环或违规后再次扫描，显示已解决。
7. 在 100 个以上 module 的项目中，首屏保持聚合且拖拽、缩放流畅。
8. 删除或破坏 Cargo.toml 后，module 图仍可用并明确显示 Cargo 分析失败。

## 发布切片

1. **架构数据：** workspace/module 图、稳定 ID、schema v3 和序列化兼容。
2. **架构风险：** SCC、扇出、分层配置、统一 Finding 和历史差异。
3. **架构 UI：** 第五分类、摘要、确定性布局、交互 Canvas 和详情。
4. **变化与行动：** 新增边/环、影响范围、源码跳转和 Agent 诊断。

每个切片都必须使用真实数据。UI 在对应数据能力完成前显示未提供状态，不填充演示节点。
