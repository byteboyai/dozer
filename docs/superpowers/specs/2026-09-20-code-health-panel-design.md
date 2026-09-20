# 代码健康度面板设计

**状态：已批准（brainstorming 会话，2026-09-20）**

## 背景

用户提出："几次重构后 Dozer 仍然出现严重问题，非职业程序员朋友的经验是核心代码超过
2 万行就容易出问题；agent 编排是次要问题（人类注意力有限），代码质量的可视化呈现
对 Dozer 来说可能是个大亮点。"

产品定位在会话中被明确收敛为两条，且明确排除第三条：

1. **预警**——让用户（甲方，可能不是职业程序员）意识到"这是个重要问题"。
2. **定位**——让用户知道具体该处理图表里的哪个文件/函数。
3. **不做"解决"**——用户明确认为解决问题是 AI Agent 擅长的事，本面板不做自动重构、
   不发起 agent 任务，只负责让人（或 agent）知道"哪里有问题"。

这与 Dozer 现有定位一致：agent 中立、站在用户侧的验收层；agent 编排本身已经是被
搁置的次要项（`dozer-builtin-agent-work-paused` 记忆）。

### 技术可行性 spike（会话中已完成）

`spike/code-analysis/`（未提交，随时可删）验证了：

- `ast-grep-core` + `ast-grep-language`（均 MIT，Rust 原生 crate，依赖内建 26 种语言的
  tree-sitter 语法但只需要用到 Rust）能在不引入 Node/Python 的前提下做结构化解析。
- 冷编译 ~19s，增量编译 ~1.2s，编译成本可接受。
- 在 dozer 自己 195 个 `.rs` 文件（约 9.6 万行）上跑通，真实发现
  `crates/dozer-app/src/app/update.rs:37` 的 `update` 函数复杂度信号为 242，是第二名
  （98）的 2.5 倍——直接印证了用户描述的"功能不断堆到一个函数里"问题，且这个文件正是
  `dozer-extension-architecture-idea` 记忆里提到的、五轮拆分后仍在增长的巨石文件。
- 全仓库扫描（debug build）耗时约 1-2 分钟，不足以支撑实时/自动刷新，只能支撑"手动
  触发"的使用模式（见下方"非目标"）。

参考了 CodeSee（已于 2024 年被 GitKraken 收购下线，其"每次 PR 自动生成变更影响地图"
的思路值得借鉴，见"未来方向"）与 [dep-tree](https://github.com/gabotechs/dep-tree)
（Go 写的依赖关系力导向图可视化工具，其"一眼看乱不乱"的可视化手法值得借鉴，但依赖
Go 工具链不能直接引入）。两者均不作为依赖集成，只借鉴产品思路。

## 目标 / 非目标

**目标：**

1. 新增独立面板 `PanelKind::CodeHealth`（挂在右侧图标栏，与现有
   `PanelKind::{Agent, Conversations, Usage}` 平级，参照 `crates/dozer-app/src/chrome/rail.rs`
   现有注册模式）。
2. 分析粒度覆盖项目 / 文件 / 函数三层，但 UI 上只用两块（健康卡片 + 问题列表）呈现，
   不做三个独立视图（见"UI 设计"）。
3. 扫描范围：当前激活项目下的全部 `.rs` 源文件（一期只做 Rust，见下）。
4. 手动触发：面板打开时展示上次扫描结果（落盘持久化），用户点"扫描"按钮才重新跑一遍；
   不做文件监听、不做自动增量刷新。
5. 数据层是一个独立 lib crate `crates/dozer-codehealth`，不碰 iced/UDS 协议，纯函数
   API（`scan_project(root: &Path) -> ProjectReport`），供 `dozer-app` 消费，也为未来
   `dozer-mcp` 复用留门（见"未来方向"第 7 条，不在本次范围内实现）。
6. 复杂度/规模阈值一期写死为常量，不做可配置项、不做设置面板。

**非目标**（brainstorming 阶段逐条问清楚、明确排除的）：

- **不做自动重构、不发起 agent 任务**。本面板只负责"预警 + 定位"，"解决"是 agent 的
  职责，产品定位里明确划清的边界。
- **不做自动增量扫描 / 面板角标预警**。全仓库扫描 1-2 分钟的性能现状不支持实时刷新；
  一期只做手动触发，自动预警留作二期（见"未来方向"第 1 条）。
- **不做依赖关系可视化图**（dep-tree 式力导向/聚类图）。一期以排行榜/数字列表为主，
  可视化图形留作二期（见"未来方向"第 2 条）。
- **不支持多语言**。一期只解析 `.rs` 文件；`ast-grep-language` 里其他 25 种语言的
  语法虽然已经是依赖的一部分，但每种语言的函数/复杂度节点定义（如 Rust 的
  `function_item`）需要单独适配，一期不做（见"未来方向"第 5 条）。
- **不做阈值可配置面板**。复杂度/规模阈值一期是代码里的写死常量。
- **不做变更影响地图（CodeSee 思路）、循环依赖检测、架构规则 lint**。均记录在"未来
  方向"，不在本次范围。

## 指标定义

### 函数级

对每个 `function_item` 节点（用 `ast-grep-core::Node::dfs()` + `.field("name")` 提取，
同 spike 已验证的方式）：

```rust
pub struct FunctionMetric {
    pub name: String,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub loc: usize,               // end_line - start_line + 1
    pub complexity_signal: usize, // 函数体内 if/match/for/while/loop/closure 节点计数
    pub severity: Severity,       // Normal / Watch / Critical，由 complexity_signal 判定
}
```

`complexity_signal` 是圈复杂度（McCabe）的粗代理，不是精确计算——数的是控制流分支
节点总数，不追求学术精确度，只作为"值不值得优先看"的排序信号。

判定阈值（写死常量，非配置项）：

| `complexity_signal` | `Severity` |
|---|---|
| `> 40` | Critical |
| `16..=40` | Watch |
| `<= 15` | Normal |

这三档参照常见静态分析工具的经验值（如 SonarQube 圈复杂度默认阈值量级），不是从
dozer 自身数据分布反推的精确值；如果一期使用后发现系统性偏差（比如"正常"函数被
频繁误判），属于可以直接改常量的调整，不需要重新设计。

### 文件级（从函数级聚合）

```rust
pub struct FileMetric {
    pub path: PathBuf,
    pub loc: usize,
    pub critical_functions: usize,
    pub watch_functions: usize,
    pub flagged: bool, // critical_functions > 0 || loc > 1000
}
```

判定：文件内存在 ≥1 个 Critical 函数，或文件总行数 > 1000 行 → 标记为问题文件。
1000 行是经验阈值（同函数级阈值一样是写死常量）。

### 项目级（规模 + 问题密度复合）

```rust
pub struct ProjectReport {
    pub total_loc: usize,
    pub total_functions: usize,
    pub critical_functions: usize,
    pub scale_tier: HealthTier,     // 由 total_loc 分档
    pub density_tier: HealthTier,   // 由 critical_functions / total_functions 分档
    pub overall_tier: HealthTier,   // max(scale_tier, density_tier)
    pub functions: Vec<FunctionMetric>, // 供问题列表渲染，按 severity 降序排列
}

pub enum HealthTier { Healthy, Watch, Critical }
```

规模分档（对应用户朋友的"2 万行"经验直觉）：

| `total_loc` | `scale_tier` |
|---|---|
| `< 10_000` | Healthy |
| `10_000..=20_000` | Watch |
| `> 20_000` | Critical |

密度分档（问题函数占比）：

| `critical_functions / total_functions` | `density_tier` |
|---|---|
| `< 2%` | Healthy |
| `2%..=5%` | Watch |
| `> 5%` | Critical |

`overall_tier` 取两个维度里较严重的一档（简单 max 规则，不做加权公式）——目的是
可解释性优先："为什么是这个等级"要能一句话讲清楚给非程序员朋友听，不需要理解权重
计算。

## 架构与数据流

### 1. `crates/dozer-codehealth`：独立数据层 crate

新建 lib crate，workspace 成员，依赖 `ast-grep-core`、`ast-grep-language`；不依赖
iced/`dozer-core::protocol`/UDS，是纯函数库（同 `dozer-core` 的定位——共享类型/逻辑，
不含 GUI 或协议细节）。

```rust
// crates/dozer-codehealth/src/lib.rs
pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport>
```

内部：递归收集 `root` 下全部 `.rs` 文件（跳过 `target/`，同 spike 的
`collect_rs_files` 逻辑）→ 逐文件 `SupportLang::Rust.ast_grep(&src)` 解析 → `dfs()`
找 `function_item` → 算 `FunctionMetric` → 聚合出 `FileMetric`/`ProjectReport`。

单文件解析失败（如语法错误/非 UTF-8）跳过该文件，不中断整体扫描（同 `transcript.rs`
"读不到就跳过"的既有错误处理哲学）。

独立成 crate 而不是直接写进 `dozer-app::extensions::codehealth` 的原因：将来
`dozer-mcp` 若要把"当前项目哪里最乱"暴露给 agent 查询（见"未来方向"第 7 条），
可以直接依赖这个 crate，不需要反向依赖 GUI crate。

### 2. `dozer-app` 侧：`extensions/codehealth/`

镜像 `extensions/usage/` 的分层惯例（`aggregate.rs` 数据整形 / `view.rs` 渲染 /
`mod.rs` 挂 `WorkspaceState`）：

```rust
#[derive(Default)]
pub struct WorkspaceState {
    report: Option<ProjectReport>,
    scanned_at: Option<i64>, // 上次扫描时间(ms epoch)，随 report 一起落盘
    scanning: bool,
}
```

扫描结果连同时间戳落盘持久化（具体存储位置/格式——沿用 `dozerd` 现有的项目级数据
目录约定，还是 `dozer-app` 本地文件——留给 plan 阶段核实现有基础设施后决定，不在此
预设）。面板打开时优先读本地持久化结果展示；没有历史记录才提示"点击开始首次扫描"。

`PanelKind::CodeHealth` 加入 `crates/dozer-app/src/chrome/rail.rs` 的
`right: vec![...]`，图标沿用现有 `icons::IconKind` 体系新增一个变体
`IconKind::SquareActivity`（Lucide [`square-activity`](https://lucide.dev/icons/square-activity)，
同现有图标"内嵌 svg + 加一个变体"的接入方式，如 `BarChart3` 之于 Usage 面板）。

### 3. 触发与消息流

参照 Usage 面板的 `Task::perform` + 阻塞线程解析模式（`scan_project` 涉及大量文件
IO + CPU 密集的 AST 遍历，不能占 iced 的 update 线程）：

```rust
enum Message {
    CodeHealthScan,           // 点"扫描"按钮触发
    CodeHealthScanned(anyhow::Result<ProjectReport>),
}
```

面板首次打开**不**自动触发扫描（与 Usage 面板"打开即自动扫"不同）——本面板一期
明确是"手动触发"模式，打开面板默认展示上次落盘结果，避免每次切换面板都要等 1-2
分钟。

### 4. UI 设计

用两块 UI 覆盖项目 / 文件 / 函数三层，不做三个独立视图：

1. **顶部健康卡片**：大号等级徽章（`overall_tier` 对应现有
   `byteui::theme::color::ColorTokens`：Healthy → `green` `#1AD585`、Watch → `cyan`
   `#47DEF0`、Critical → `red` `#FF6E6E`；不用 `gold`——CLAUDE.md 明确 `gold` 是甲方
   动作专属色）+ 一句话摘要（"核心代码 {total_loc} 行，{critical_functions} 个函数
   存在明显结构问题"）+ "扫描"按钮 + 上次扫描时间戳。
2. **问题列表**：`ProjectReport.functions`（已按 severity 降序排列）渲染成列表，
   按文件路径分组（天然覆盖文件级聚合，不需要单独的文件级视图），每行显示函数名 +
   `complexity_signal` + `loc`；点击跳转到 Files 面板并定位到该函数所在行（具体复用
   哪个现有的"打开文件 + 定位到行"机制，仓库里没有找到明确的既有跨面板跳转先例，
   留给 plan 阶段核实——如果没有现成基础设施，这条跳转本身要在实现计划里当一个
   独立子任务估）。
3. 加载中占位态："扫描中…{已处理文件数}/{总文件数}"（若能拿到进度）或简单的
   "扫描中…"。
4. 空态：项目下没有任何 `.rs` 文件时，提示"这个项目还没有可分析的 Rust 代码"。

### 5. 错误处理

延续既有的"读不到/解析不动就跳过，不报错"哲学：

- 目录不存在/无权限读 → 该次扫描直接返回空 `ProjectReport`（`total_functions = 0`），
  UI 走空态，不弹错误框。
- 单文件解析失败 → 跳过该文件，日志记录（`tracing::warn`），不中断整体扫描、不计入
  `total_loc`/`total_functions`。
- 扫描过程中项目被删除/移动 → `Task::perform` 返回 `Err`，面板展示"扫描失败，请
  重试"，保留上一次成功的落盘结果不被覆盖。

## 测试策略

- `dozer-codehealth`：用内联构造的 fixture `.rs` 文件（临时目录，同 spike 验证过的
  真实解析路径）覆盖：
  - `complexity_signal` 计数正确性（构造已知控制流节点数的函数）。
  - 三档 `Severity`/`HealthTier` 判定边界值（`complexity_signal` 恰好 15/16/40/41，
    `total_loc` 恰好 9999/10000/20000/20001，密度比例边界）。
  - 文件解析失败（语法错误文件）不中断整体扫描，其余文件正常统计。
  - 空项目（无 `.rs` 文件）返回全零 `ProjectReport`，不 panic。
  - `overall_tier` 取两个维度较严重一档的规则（构造 scale=Healthy/density=Critical
    等组合验证 max 逻辑）。
- `dozer-app::extensions::codehealth`：`WorkspaceState` 落盘/读取往返正确；扫描中
  状态位正确置位/复位。
- 面板渲染（健康卡片配色、问题列表分组、点击跳转）留给 `cargo run -p dozer-app`
  人工验收，同现有面板惯例。

## 依赖变更

新增 `crates/dozer-codehealth`，依赖：

```toml
ast-grep-core = "0.45"
ast-grep-language = "0.45"
```

（精确版本号以 spike 阶段 `cargo add` 解析到的 `0.45.3` 为准，实现时按 workspace
惯例走 `cargo add` 由 cargo 自行解析兼容版本，不手工写死小版本号。）

均为 MIT 协议，纯 Rust，不引入 Node/Python 运行时依赖，符合 CLAUDE.md 架构裁决。

## 未来方向

以下方向在本次 brainstorming 中讨论过，**均明确排除在本次范围外**，记录供后续
独立立项参考，不代表已排期：

1. **自动预警**：文件监听（或 agent 会话结束时触发）+ 后台增量重扫（只重扫变更
   文件、复用/缓存已解析的 AST）+ 面板图标角标（红点）提醒用户主动查看。前提是先
   把增量缓存的架构设计清楚，现在的"全量扫描 1-2 分钟"性能不支持这个方向。
2. **依赖关系可视化**：参考 [dep-tree](https://github.com/gabotechs/dep-tree) 的
   力导向/聚类图思路——结构清晰的代码呈现分离的簇，纠缠的代码呈现压缩的团块，不需要
   看懂任何数字就能感知"乱不乱"，比数字排行榜更贴合"让非程序员也看懂预警"的诉求。
   数据源可以用 `ast-grep-core` 提取 `use`/`mod` 关系，图形渲染在 `iced::widget::canvas`
   里自己实现一个简化版布局算法，不引入外部 Go/Node 工具链。
3. **变更影响地图**（参考 CodeSee 已下线的 Review Map 功能）：每次 agent 会话/diff
   结束后自动生成一张"这次改动碰了哪些文件、它们跟其他模块什么关系"的视图，比静态
   全量快照更贴合 Dozer 的验收场景（"这次 agent 到底动了哪里"）。
4. **循环依赖检测 + 架构规则 lint**：在依赖关系图（第 2 条）基础之上，参考
   dep-tree 的 `allow`/`deny` 依赖白名单思路，让用户（或未来的规格文档）声明"哪些
   模块不该互相依赖"，面板检测违规。这条把"预警/定位"从代码质量扩展到架构治理。
5. **多语言支持**：`ast-grep-language` 已经内建 26+ 种语言的语法，按 dozer 用户
   项目实际语言分布（最可能是 TS/JS、Python）逐个补 `function_item` 等价节点定义
   和复杂度判定阈值。
6. **阈值可配置**：项目级自定义警戒线（用户可能觉得自己项目"复杂点正常"），需要
   配置 UI + 持久化。
7. **通过面板能力注册表暴露给 agent**：对接
   `docs/superpowers/specs/2026-09-18-panel-capability-tools-for-agents-design.md`
   （目前状态"待审阅"，本设计不依赖它落地）——让 agent 能主动查询"当前项目哪里
   最乱"，把"预警 + 定位"和"agent 解决"真正闭环，而不需要用户手动把面板结果讲给
   agent 听。`crates/dozer-codehealth` 独立成 crate 正是为这条路径留的架构
   门（见"架构与数据流"第 1 节）。
