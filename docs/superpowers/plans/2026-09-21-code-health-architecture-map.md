# 代码健康度架构地图 Implementation Plan

> **For agentic workers:** 按 Task 顺序实施。每个任务先补最小失败测试，再实现和运行定向测试。不要改写已有复杂度算法，不要把外部图形 CLI 变成运行时依赖。

**Goal:** 在代码健康度下一迭代中，从 Cargo workspace 与 Rust AST 生成可交互架构图，检测循环依赖、依赖枢纽和分层边界违规，并展示本轮新增关系与影响范围。

**Architecture:** `dozer-codehealth` 复用单次 AST 扫描构建与 UI 无关的 `ArchitectureReport`，用图算法生成风险发现；现有快照继续持久化整个版本化报告；`dozer-app` 计算可见子图和确定性布局，通过 iced Canvas 渲染与交互。外部 crate、Graphviz、Mermaid 和 `cargo-modules` 不参与运行时。

**Tech Stack:** Rust、`cargo_metadata`、`petgraph`、ast-grep/tree-sitter、globset、serde、iced 0.14 Canvas。

**Spec:** `docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md`

## Global Constraints

- 当前工作区已有未提交代码健康修复和用户文件改动；只提交本计划所属文件，不使用 `git add -A`。
- `crates/dozer-app/src/extensions/files/view.rs` 和 `crates/byteui/assets/icons/circle-x.svg` 属于用户现有改动，不得修改、清理或纳入提交。
- `dozer-codehealth` 保持纯分析库，不依赖 iced、rusqlite、dozerd 或 app 状态。
- 架构提取必须复用每个 Rust 文件已有 AST；禁止为了依赖图再次全量解析源码。
- 新字段使用 serde 默认值；schema v2 报告必须可读，旧报告不得被解释为“架构健康”。
- 所有边和发现必须能回溯到证据；无法解析的关系只计数，不猜测目标。
- 首版不做函数调用图，不启动 Graphviz/Mermaid/cargo-modules 子进程。
- UI 继续使用“详情在左、分类导航在右”的既有布局。
- 图风险不能只靠颜色表达；节点、边和列表必须有文字状态。
- 手动扫描语义不变；不增加文件监听、自动重构或 CI 门禁。

---

### Task 1：定义 schema v3 架构模型与兼容读取

**Files:**

- Create: `crates/dozer-codehealth/src/architecture.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`
- Modify: `crates/dozer-codehealth/src/report.rs`
- Modify: `crates/dozer-codehealth/src/scan_metadata.rs`

**Produces:** 与具体图库无关、可 serde 的架构节点、边、证据、循环和状态类型。

- [ ] 在旧 schema v2 fixture 上增加测试：缺少 `architecture` 字段仍可读取，得到 `ArchitectureReport::default()`，`schema_version` 保留 2。
- [ ] 将 `SCHEMA_VERSION` 升为 3；保留 v1/v2 读取路径。
- [ ] 定义 `ArchitectureReport`、`ArchitectureStatus`、`ArchitectureNode`、`ArchitectureNodeKind`、`ArchitectureEdge`、`ArchitectureEdgeKind`、`ArchitectureEvidence` 和 `DependencyCycle`。
- [ ] 给 `ProjectReport` 增加 `#[serde(default)] pub architecture: ArchitectureReport`。
- [ ] 默认架构状态使用 `NotApplicable`，避免旧报告显示为“完整且零风险”。
- [ ] 为所有枚举设置稳定的 `snake_case` serde 名称。
- [ ] 测试空报告、完整架构报告和 v2 fixture 的 JSON round-trip。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-codehealth architecture scan_metadata report::v2_tests
```

---

### Task 2：解析 Cargo workspace 与 crate 依赖

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/dozer-codehealth/Cargo.toml`
- Create: `crates/dozer-codehealth/src/cargo_architecture.rs`
- Modify: `crates/dozer-codehealth/src/architecture.rs`
- Modify: `crates/dozer-codehealth/src/report.rs`

**Produces:** workspace member、内部 crate 依赖和可选直接外部依赖。

- [ ] 先检查 workspace 是否已有兼容版本；以 workspace dependency 方式加入 `cargo_metadata`，禁止重复版本。
- [ ] 用临时 workspace fixture 写测试：两个 member、一个 path dependency、一个第三方 dependency。
- [ ] 从扫描根目录查找 Cargo manifest；缺失时返回可降级结果，不让整个扫描失败。
- [ ] 调用 `cargo_metadata` 时关闭完整依赖展开，避免下载依赖；记录 manifest 相对路径作为 Cargo 边证据。
- [ ] workspace member 建内部 crate 节点；直接第三方依赖建 external 节点并标记 `external=true`。
- [ ] 聚合重复 crate 边，边 ID 只使用 kind/from/to，证据列表稳定排序并去重。
- [ ] metadata 命令失败时将架构状态设为 `Partial` 并记录可展示错误；不得丢弃后续 module 图。
- [ ] 验证 target-specific、optional、renamed dependency 不会造成节点 ID 冲突。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-codehealth cargo_architecture
```

---

### Task 3：从共享 AST 构建 module 图

**Files:**

- Create: `crates/dozer-codehealth/src/module_architecture.rs`
- Modify: `crates/dozer-codehealth/src/function_metric.rs`
- Modify: `crates/dozer-codehealth/src/report.rs`
- Modify: `crates/dozer-codehealth/src/architecture.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`

**Produces:** 文件 module、内联 module 和可证明的 `use` 依赖边。

- [ ] 把一次文件分析需要的结构、UI 与架构结果收敛到内部 `AnalyzedRustFile`，调用方只创建一次 AST root。
- [ ] 为 crate root、`foo.rs`、`foo/mod.rs`、`mod foo;` 和内联 module 建稳定 qualified name。
- [ ] 解析 `crate::`、`self::`、`super::`、workspace crate 前缀、别名和嵌套 use tree。
- [ ] 把 item 级路径归并到当前已知的最深 module；解析不到的目标只增加 `unresolved_edges`。
- [ ] 每条 module use 证据记录相对路径、1-based 行号和短 snippet；证据稳定排序并去重。
- [ ] 聚合同一 from/to 的多条 use 为一条边。
- [ ] 先只记录 `cfg` 属性文本，不尝试模拟所有 feature 组合。
- [ ] 测试内联 module、跨文件 module、self/super、grouped use、rename、pub use、glob use 和无法解析路径。
- [ ] 加回归测试证明一次扫描对每个 Rust 文件只调用一次解析入口。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-codehealth module_architecture report::scan_tests function_metric
```

---

### Task 4：图算法、配置和架构发现

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/dozer-codehealth/Cargo.toml`
- Modify: `crates/dozer-codehealth/src/architecture.rs`
- Modify: `crates/dozer-codehealth/src/discovery.rs`
- Modify: `crates/dozer-codehealth/src/finding.rs`
- Modify: `crates/dozer-codehealth/src/report.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`

**Produces:** 循环、扇入/扇出、层级违规和统一架构 Finding。

- [ ] 以 workspace dependency 方式加入 `petgraph`；持久化结构不得暴露 petgraph 类型或索引。
- [ ] 扩展 `.dozer/code-health.toml`：解析阈值、影响深度和 layer 数组；使用已有 `toml`/`globset`，配置错误必须可展示。
- [ ] 构造内部节点子图，计算 fan-in、fan-out 并写回节点。
- [ ] 用 SCC 检测 crate/module 环和自环；cycle ID 对排序后的节点 ID 做稳定哈希。
- [ ] 默认 `max_fan_out=8`、`critical_fan_out=15`；验证项目配置覆盖。
- [ ] 按配置顺序将节点映射到 layer；检测未知 layer、重叠 match 和禁止依赖方向。
- [ ] `FindingCategory` 增加 `Architecture`；`FindingEvidence` 增加 cycle/hub/boundary 三种证据。
- [ ] 生成三个稳定规则 ID，架构发现路径优先使用来源边证据或节点源文件。
- [ ] `metric_value()` 对 hub 返回 fan_out；cycle 和 boundary 没有线性指标时返回 `None`。
- [ ] 将架构风险计入 Finding diff 和热点排序，但不要让影响范围本身成为 Finding。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-codehealth architecture finding diff discovery
```

---

### Task 5：架构差异与变更影响范围

**Files:**

- Create: `crates/dozer-codehealth/src/architecture_diff.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/aggregate.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`

**Produces:** 节点、边、循环变化和最多三层的反向依赖影响范围。

- [ ] 定义 `ArchitectureDiff`，用稳定 ID 计算 added/removed nodes、edges、cycles，结果按 ID 排序。
- [ ] 无上一份 schema v3 架构报告时返回显式 `NoBaseline`，不要把所有边显示为本轮新增。
- [ ] 从 Git dirty 路径和相邻报告中有变化的 module 得到 impact seeds。
- [ ] 沿内部反向依赖边做有深度上限的 BFS，区分直接和间接影响节点，并记录最短距离。
- [ ] 深度从项目配置读取，默认 3，并限制访问节点总数防止异常大图拖慢 UI。
- [ ] `PanelState` 增加架构差异与影响范围；缓存加载和手动扫描都使用正确的历史基线。
- [ ] 为新增边、移除边、首次扫描、旧 schema、深度限制、环图 BFS 和稳定排序写测试。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-codehealth architecture_diff
env RUSTC_WRAPPER= cargo test -p dozer-app codehealth::git_hotspots codehealth::tests
```

---

### Task 6：确定性可见子图与布局

**Files:**

- Create: `crates/dozer-app/src/extensions/codehealth/architecture_view_model.rs`
- Create: `crates/dozer-app/src/extensions/codehealth/architecture_layout.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`

**Produces:** 与 iced 绘制解耦的过滤、折叠和坐标布局。

- [ ] 定义 `ArchitectureLevel::{Crate, Module}`、过滤器、展开节点集合和选中对象。
- [ ] 默认可见图只包含 workspace crate 与一级 module；第三方和无风险节点按开关过滤。
- [ ] 将被折叠的后代依赖聚合到最近可见祖先，去掉聚合后自环并合并重复边。
- [ ] 实现从左到右的确定性分层布局：拓扑层级、SCC 压缩、同层稳定 ID 排序、固定节点间距。
- [ ] 输出 `LayoutNode` 矩形、`LayoutEdge` 折线/状态和整体边界；不得把像素坐标写回扫描报告。
- [ ] 强制 80 节点/160 边可见上限，输出被聚合或截断数量。
- [ ] 为同输入同坐标、节点不重叠、循环图、折叠聚合、过滤和上限写纯逻辑测试。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-app codehealth::architecture_view_model codehealth::architecture_layout
```

---

### Task 7：实现架构分类与交互 Canvas

**Files:**

- Create: `crates/dozer-app/src/extensions/codehealth/architecture_canvas.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/view_model.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`

**Produces:** 第五分类、摘要、图画布、选择详情和风险列表。

- [ ] `CodeHealthCategory` 增加 `Architecture`，右侧顺序为总览/结构/架构/UI 一致性/扫描范围。
- [ ] 新增 WorkspaceState：层级、风险过滤、第三方显示、展开集合、选择项；项目切换时恢复默认。
- [ ] 新增 Message：切换层级/过滤、选择节点/边、展开折叠、聚焦风险和重置视图。
- [ ] Canvas 绘制节点、聚合边、箭头、状态徽标与选中态；新增/循环/违规都显示文字或图例。
- [ ] 实现空白拖拽平移、0.5–2.5 滚轮缩放、命中测试、单击选择、双击展开和适配窗口。
- [ ] 页头和摘要展示节点、边、循环、违规、新增关系数量；无基线时明确显示首次扫描。
- [ ] 选中详情展示 qualified name、LOC、fan-in/out、layer、依赖列表、证据和影响距离。
- [ ] 风险列表按新增/恶化、严重度、Git 热点排序；点击后展开祖先、选择并居中。
- [ ] 节点“打开源码”和边证据复用现有 `Message::OpenLocation`。
- [ ] 旧报告、非 Rust、Cargo 失败、部分架构、超大图都使用 spec 定义的准确状态文案。
- [ ] 提取缩放 clamp、viewport reset、hit-test 为纯函数并写单元测试；绘制结果做人工验收。

**Verify:**

```bash
env RUSTC_WRAPPER= cargo test -p dozer-app codehealth
env RUSTC_WRAPPER= cargo check -p dozer-app
```

---

### Task 8：源码跳转、Agent 诊断与全链路验收

**Files:**

- Modify: `crates/dozer-app/src/extensions/codehealth/view_model.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md`（仅记录确认后的实现偏差）
- Modify: `docs/superpowers/plans/2026-09-21-code-health-architecture-map.md`（勾选实际完成项）

**Produces:** 从图到源码、从风险到 Agent 输入区的完整行动路径。

- [ ] 节点打开其声明文件；边默认打开第一条证据，详情可选择其他证据。
- [ ] 扩展 `AnalyzeFinding` 文本：节点/边、依赖方向、证据、变化、影响范围、配置预期和验收目标。
- [ ] 保持现有安全行为：诊断文本只写入当前 Agent 输入区，不附加回车，不自动修改代码。
- [ ] 在没有活动 Agent 会话时沿用现有无操作或提示策略，不创建隐藏会话。
- [ ] 使用临时 fixture 完成三次扫描：建立基线、制造环/违规、修复；核对新增和已解决。
- [ ] 扫描 Dozer 自身并记录节点数、边数、未解析边、架构额外耗时和 Canvas 首屏规模。
- [ ] 人工核对右侧分类顺序、图例、缩放平移、展开折叠、风险定位和源码跳转。
- [ ] 运行完整相关测试、格式检查和 diff 检查；确认未触碰用户已有文件改动。

**Verify:**

```bash
cargo fmt --all --check
env RUSTC_WRAPPER= cargo test -p dozer-codehealth
env RUSTC_WRAPPER= cargo test -p dozer-core code_health
env RUSTC_WRAPPER= cargo test -p dozerd code_health
env RUSTC_WRAPPER= cargo test -p dozer-app codehealth
env RUSTC_WRAPPER= cargo check -p dozer-app
git diff --check
```

## Recommended Commit Boundaries

1. `feat(codehealth): add versioned architecture report model`
2. `feat(codehealth): extract cargo workspace dependencies`
3. `feat(codehealth): build module graph from shared rust ast`
4. `feat(codehealth): detect architecture cycles and boundaries`
5. `feat(codehealth): diff architecture and calculate impact scope`
6. `feat(codehealth): add deterministic architecture graph layout`
7. `feat(codehealth): render interactive architecture map`
8. `feat(codehealth): connect architecture risks to source and agents`

提交时逐项 `git add` 本任务文件。不要使用 `git add -A`，不要纳入 `crates/dozer-app/src/extensions/files/view.rs` 或 `crates/byteui/assets/icons/circle-x.svg`。
