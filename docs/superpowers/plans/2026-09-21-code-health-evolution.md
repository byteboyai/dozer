# 代码健康度演进 Implementation Plan

> **For agentic workers:** 按 Task 顺序实现。每个 Task 完成后运行该 Task 的定向测试；不要重写现有 v1 指标或顺手清理无关代码。

**Goal:** 将现有代码健康板块升级为可解释的项目健康工具：明确扫描范围，保存历史快照，展示本轮新增/已解决风险，用 Git 变动频率排序热点，并实现 Figma 已确认的“详情在左、分类导航在右”页面。

**Architecture:** `dozer-codehealth` 产出带版本、扫描范围和统一发现项的报告；`dozerd` 保存最近 30 份快照并保留最新报告兼容接口；`dozer-app` 计算/消费相邻报告差异和 Git 热点，渲染总览、结构、UI 一致性、扫描范围四个分类。

**Tech Stack:** Rust、ast-grep/tree-sitter、`ignore`、git CLI、rusqlite、serde、iced 0.14。

**Spec:** `docs/superpowers/specs/2026-09-21-code-health-evolution-design.md`

**Figma:** https://www.figma.com/design/NXfLQp5XQk1kF7Ohls2EbX/Dozer-Phase-1-UI?node-id=176-146

## Global Constraints

- 保留现有 `ProjectReport` 字段和 v1 JSON 读取能力，迁移期间新增字段使用 `#[serde(default)]` 或显式兼容类型。
- `dozer-codehealth` 继续保持纯分析库，不依赖 iced、UDS、rusqlite 或应用状态。
- 现有 `complexity_signal` 在 UI 中改称“控制流信号”，不要暗示它是标准圈复杂度。
- 所有健康状态除颜色外必须有文字；变化值必须带 `+`/`-` 或“新增/已解决”。
- 详情内容区位于分类导航左侧；不要恢复当前实现中的导航在左布局。
- 不修改用户当前未提交的 `crates/dozer-app/src/extensions/files/view.rs`。
- 不新增自动扫描、CI 门禁或自动代码修改。
- 每个数据库 schema 变化必须兼容已有 `dozer.db`。

---

### Task 1：报告 v2 类型与旧报告兼容

**Files:**

- Modify: `crates/dozer-codehealth/src/report.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`
- Create: `crates/dozer-codehealth/src/finding.rs`
- Create: `crates/dozer-codehealth/src/scan_metadata.rs`

**Produces:** `SCHEMA_VERSION`、`ScanMetadata`、`ScanStatus`、`SkippedFile`、`Finding`、`GitSnapshot`，以及带 serde 默认值的 v2 `ProjectReport`。

- [ ] 写旧版最小 JSON fixture，确认新增字段前可反序列化。
- [ ] 定义 `SCHEMA_VERSION: u32 = 2` 和扫描范围类型；枚举使用 `snake_case` serde 名称。
- [ ] 定义统一 `Finding` 外壳和规则证据类型，先覆盖现有结构复杂度、颜色、间距、字体、嵌套、回调和重复结构。
- [ ] 给 `ProjectReport` 增加 v2 字段并为旧 JSON 提供默认值；不要删除现有字段。
- [ ] 将现有指标转换为 `findings`，保证旧字段与统一发现项来自同一次扫描结果。
- [ ] 测试旧 JSON、空项目报告和包含全部发现类型的 v2 JSON round-trip。

**Verify:**

```bash
cargo test -p dozer-codehealth report finding scan_metadata
```

---

### Task 2：可信文件发现与扫描范围

**Files:**

- Modify: `crates/dozer-codehealth/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/dozer-codehealth/src/discovery.rs`
- Modify: `crates/dozer-codehealth/src/report.rs`

**Produces:** 遵守 ignore 规则的文件清单，以及完整/部分/失败扫描状态。

- [ ] 用 `cargo add ignore` 添加依赖。
- [ ] 用 `ignore::WalkBuilder` 替换手写 `collect_rs_files`，遵守 `.gitignore`/`.ignore` 并跳过构建目录。
- [ ] 读取可选 `.dozer/code-health.toml`；若项目尚无 TOML 配置解析依赖，优先复用 workspace 已有依赖，禁止手写不完整 TOML parser。
- [ ] 对每个候选文件记录语言、分析结果或跳过原因；路径统一保存为相对项目根目录的规范化路径。
- [ ] 将非 UTF-8、读取失败和解析失败从静默跳过改为 `SkippedFile`。
- [ ] 没有受支持代码时返回成功报告和明确范围，不再等同于“尚未扫描”。
- [ ] 增加 `.gitignore`、生成文件、非 UTF-8、解析错误和空目录测试。

**Verify:**

```bash
cargo test -p dozer-codehealth discovery report::tests::scan_project
```

---

### Task 3：单次解析与扫描性能基线

**Files:**

- Modify: `crates/dozer-codehealth/src/function_metric.rs`
- Modify: `crates/dozer-codehealth/src/ui_metrics.rs`
- Modify: `crates/dozer-codehealth/src/report.rs`
- Create: `crates/dozer-codehealth/benches/scan_project.rs`（仅当 workspace 已有 benchmark 基础设施；否则创建可重复运行的 ignored test）

- [ ] 重构函数分析入口以接收已经解析的 AST/root，移除同一文件在 `report.rs` 与 `functions_in_source` 中各解析一次的现状。
- [ ] 保持 `Patterns` 在文件循环外只编译一次。
- [ ] 建立 10 万行级 fixture 或可重复的本仓库基线命令，记录 debug/release 扫描时间与峰值发现数。
- [ ] 验证重构前后现有 fixture 的所有指标一致。
- [ ] 若 release 基线超过 15 秒，再评估按文件并行；未超标则停止优化。

**Verify:**

```bash
cargo test -p dozer-codehealth
cargo test -p dozer-codehealth --release -- --ignored scan_performance_baseline
```

---

### Task 4：稳定发现 ID 与报告差异

**Files:**

- Create: `crates/dozer-codehealth/src/diff.rs`
- Modify: `crates/dozer-codehealth/src/finding.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`

**Produces:** `diff_reports(previous, current) -> ReportDiff`。

- [ ] 实现路径、符号和结构签名归一化；发现 ID 不包含行号、当前严重度或当前指标值。
- [ ] 为没有符号的字面量发现生成邻近结构指纹，避免简单插入行导致 ID 全变。
- [ ] 定义 `FindingChange::{New, Persisting, Worsened, Improved, Resolved}` 和汇总计数。
- [ ] 用哈希表按 ID 比较；同 ID 的规则证据决定改善或恶化。
- [ ] 测试行号移动、函数增加分支、严重度跨档、问题修复和首次扫描。

**Verify:**

```bash
cargo test -p dozer-codehealth diff finding_id
```

---

### Task 5：历史快照数据库与协议

**Files:**

- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozerd/src/code_health.rs`
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Produces:** 保存快照、读取最近 N 份以及读取最新报告的协议接口。

- [ ] 新建 `code_health_report_snapshots` 表和项目/时间索引；`CREATE TABLE IF NOT EXISTS` 必须可在旧数据库直接运行。
- [ ] 保存报告时事务内插入快照、更新现有 `code_health_reports` 最新行、删除该项目第 31 份及更旧记录。
- [ ] 扩展协议加入 `ListCodeHealthReports { project_id, limit }`，限制 `limit` 合理上限。
- [ ] `CodeHealthReportInfo` 增加 schema/git 元数据时必须对旧协议 JSON 使用 serde 默认值。
- [ ] 测试连续保存、倒序读取、项目隔离、30 份清理和旧表升级。
- [ ] 更新真实 daemon 集成测试，验证 client/server round-trip。

**Verify:**

```bash
cargo test -p dozerd code_health
cargo test -p dozer-core code_health
cargo test -p dozer-client --test against_real_daemon code_health
```

---

### Task 6：Git 快照与热点排序

**Files:**

- Create: `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/aggregate.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`

**Produces:** 当前 Git 元数据和 `Vec<HotspotView>`。

- [ ] 复用仓库已有 git 命令执行抽象；不存在合适抽象时，用单次批量 `git log --since=30.days --name-only` 构建文件修改计数。
- [ ] 获取 HEAD、分支和 dirty 状态；Git 不可用返回 `None`，不得让扫描失败。
- [ ] 扫描前加载最近报告；扫描后在 blocking 任务中计算报告差异和热点。
- [ ] 按“新增/恶化、严重度、近期修改、指标增量、当前值”排序，最后用路径和规则 ID 保证稳定并列顺序。
- [ ] `WorkspaceState` 保存当前报告、上一报告、diff、hotspots 和保存错误状态。
- [ ] 测试无 Git、首次扫描、dirty 工作区和排序规则。

**Verify:**

```bash
cargo test -p dozer-app --lib codehealth::git_hotspots
cargo test -p dozer-app --lib codehealth::tests
```

---

### Task 7：分类状态与新 UI 纯逻辑

**Files:**

- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`
- Create: `crates/dozer-app/src/extensions/codehealth/view_model.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`

- [ ] 将分类扩为 `Overview`、`Structure`、`UiConsistency`、`ScanScope`，默认 `Overview`。
- [ ] 建立纯 `view_model`：变化卡片文案、热点行、范围摘要、准确空状态、默认筛选。
- [ ] 旧报告映射为“旧版报告，重新扫描可查看变化与范围”。
- [ ] 将 UI 文案中的 `complexity` 改为“控制流信号”。
- [ ] 对不适用的 iced UI 规则返回 `NotApplicable`，不得映射为 Healthy。
- [ ] 为所有 view model 分支写单元测试；渲染层只消费已经格式化的数据。

**Verify:**

```bash
cargo test -p dozer-app --lib codehealth::view_model
cargo test -p dozer-app --lib codehealth::view::tests
```

---

### Task 8：实现 Figma 总览与右侧分类导航

**Files:**

- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`
- Modify: `crates/dozer-app/src/app/view.rs`（仅在布局接线需要时）
- Modify: `crates/dozer-app/src/panel_layouts.rs`（仅在默认分栏比例需要时）

- [ ] 按 Figma 实现总览：标题/分支、状态卡、本次变化、优先处理、扫描可信度、Agent 入口。
- [ ] 内容 pane 排在左，分类 list pane 排在右，最右 Rail 不变；镜像布局仍沿用通用 pane 规则。
- [ ] 总览变化数据缺失时显示“首次扫描”，禁止用零冒充没有变化。
- [ ] 热点只渲染前 50 项并提供筛选后的继续查看方式。
- [ ] 结构和 UI 一致性页复用现有发现行与点击定位行为。
- [ ] 范围页可展开跳过原因和文件列表。
- [ ] 扫描中继续展示旧结果，同时明确标注旧结果时间。
- [ ] 保存失败与部分扫描使用独立提示，不复用通用“扫描失败”。

**Manual verify:**

```bash
cargo run -p dozer-app
```

核对 Figma 节点 `176:146`：详情区在左，分类导航在右；颜色、字号、间距使用 `byteui` token；窗口缩放和 pane 拖动无裁切。

---

### Task 9：Agent 诊断上下文入口

**Files:**

- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: 现有 agent composer/新会话入口的最小必要文件（实现前用 `rg` 定位，禁止另建并行会话系统）

- [ ] 新增 `AnalyzeFinding` 消息，携带 finding ID，不直接携带可被 UI 篡改的完整 prompt。
- [ ] update 时从当前报告解析 finding，生成包含位置、证据、变化、热点原因、验收目标和扫描局限的文本。
- [ ] 将文本送入现有 agent 输入区或创建草稿；用户仍需主动发送。
- [ ] 报告更新后找不到 finding ID 时显示可恢复错误。
- [ ] 测试诊断文本不包含绝对路径外的敏感数据，不包含自动编辑命令。

**Verify:**

```bash
cargo test -p dozer-app --lib codehealth analyze_finding
```

---

### Task 10：整体验收与文档同步

**Files:**

- Modify: `docs/superpowers/specs/2026-09-21-code-health-evolution-design.md`（只记录实现中确认的偏差）
- Modify: `docs/superpowers/plans/2026-09-21-code-health-evolution.md`（勾选完成项）

- [ ] 运行格式化和定向测试：

```bash
cargo fmt --all -- --check
cargo test -p dozer-codehealth
cargo test -p dozerd code_health
cargo test -p dozer-core code_health
cargo test -p dozer-app --lib codehealth
```

- [ ] 使用至少两个 fixture 项目验收：Rust 项目、无 Rust 或混合语言项目。
- [ ] 连续扫描三次验证首次、恶化、修复三条路径。
- [ ] 断开 Git 上下文或使用非 Git 目录验证降级行为。
- [ ] 检查旧数据库与旧报告升级，不要求用户清库。
- [ ] 手工核对所有状态不只依赖红/黄/绿颜色表达。
- [ ] 记录实际 10 万行项目扫描耗时；只有超过 spec 目标才创建后续性能任务。

## Recommended Commit Boundaries

1. `feat(codehealth): add versioned scan metadata and findings`
2. `feat(codehealth): respect ignore rules and report scan scope`
3. `perf(codehealth): parse each source file once`
4. `feat(codehealth): diff reports with stable finding ids`
5. `feat(dozerd): persist code health report history`
6. `feat(codehealth): rank change hotspots with git churn`
7. `feat(codehealth): add overview and scan scope view models`
8. `feat(codehealth): implement reviewed health dashboard layout`
9. `feat(codehealth): prepare findings for agent analysis`

不要把用户已有的 `crates/dozer-app/src/extensions/files/view.rs` 改动纳入任何提交。
