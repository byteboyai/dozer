# 代码健康度演进设计：可信扫描、变化与风险热点

**状态：已批准（用户确认 Figma，2026-09-21）**

**关联设计：** [Figma「S-CodeHealth 代码健康 · 本次变化与风险热点」](https://www.figma.com/design/NXfLQp5XQk1kF7Ohls2EbX/Dozer-Phase-1-UI?node-id=176-146)

**关联 spec：**

- `2026-09-20-code-health-panel-design.md`：Rust 结构复杂度、手动扫描与最新报告落盘。
- `2026-09-20-code-health-panel-ui-optimization-design.md`：规模/密度解释与问题排行榜。
- `2026-09-20-ui-complexity-detection-design.md`：iced UI 一致性六维检测。

本文是现有代码健康板块的下一阶段增量设计。已有指标与发现列表继续保留；本阶段把产品主问题从“当前有哪些静态问题”推进到“这轮 vibe coding 是否让项目恶化，以及现在最应该处理哪里”。

## 背景与问题

现有实现已经完成扫描、缓存、分类展示和文件跳转，但仍有四个影响用户判断的问题：

1. `scan_project` 只收集 `.rs`，仅明确排除 `target/`，单文件读取失败会静默跳过；用户无法区分“项目健康”和“没有被分析”。
2. `code_health_reports` 每个项目只保留一行，重新扫描覆盖旧报告，无法回答本轮新增、持续存在和已解决的问题。
3. 排行榜主要按静态复杂度排序。一个历史遗留的大函数会长期霸榜，而本轮新增并且持续频繁修改的风险可能排在后面。
4. 项目总行数直接参与最高健康档位，大项目容易天然得到“警戒”；规模适合作为上下文，不足以单独证明代码失控。

本阶段围绕三个产品原则设计：

- **先说明分析了什么，再给健康结论。**
- **优先展示变化，而非重复陈列全部存量债务。**
- **排序必须帮助用户决定下一步动作。**

## 目标

1. 为每次扫描生成可解释的范围报告：识别语言、已分析文件、跳过文件与原因、排除规则、耗时、扫描完整度。
2. 保存有限历史快照，并在同一项目相邻两次扫描之间计算新增、持续存在和已解决的发现。
3. 将 Git 近期修改频率与静态风险结合，形成可解释的风险热点排序。
4. 将代码健康入口改为四个分类：总览、结构复杂度、UI 一致性、扫描范围。
5. 总览首先展示本次变化、优先处理项和扫描可信度；具体发现继续支持点击定位文件。
6. 保持手动扫描，不在本阶段引入文件监听或持续后台分析。

## 非目标

- 不实现自动重构或自动修改代码。“交给 Agent 分析”只生成结构化上下文并打开现有 agent 工作流，不允许静默改代码。
- 不承诺完整的跨语言语义复杂度分析。本阶段为扫描范围建立多语言统计能力，结构复杂度仍以 Rust 为首个完整适配器。
- 不推出不可解释的单一 0–100 健康分。规模、变化、复杂度、热点和扫描完整度分别展示。
- 不把存量问题作为默认质量门禁。本阶段只在 UI 展示差异，不新增 CI 阻断。
- 不做依赖图、循环依赖和架构规则 lint。
- 不在本阶段实现阈值设置页面；配置先采用项目文件。

## 用户模型与核心流程

主要用户是使用 AI 辅助开发、无法持续人工审阅全部代码的项目负责人。

核心流程：

1. 用户完成一轮 agent 改动后打开代码健康面板。
2. 面板读取最近一次报告，用户点击“重新扫描”。
3. 扫描完成后，总览显示“本轮新增风险 / 已解决风险 / 代码量变化 / 函数变化”。
4. 用户从“优先处理”进入具体文件，或将结构化诊断交给 agent。
5. 用户修改后再次扫描，确认风险是否消失。

## 信息架构与 UI

### 总体布局

代码健康面板沿用左右配对面板：

- **左侧内容区（428px）：** 当前分类的详情。
- **右侧分类导航（336px）：** 总览、结构复杂度、UI 一致性、扫描范围。
- **最右图标栏（48px）：** 保持现有 Rail 行为。

分类导航必须位于详情右侧。该布局已经在 Figma 中由用户确认。

### 总览

从上到下依次为：

1. 标题、当前 Git 分支与工作区状态、重新扫描按钮。
2. 状态卡：使用“健康 / 需要关注 / 警戒”文字，附上次扫描时间。
3. 本次变化：代码行、函数数、新增风险；正负变化必须带符号，不只依赖颜色。
4. 优先处理：默认按热点分数排序，展示规则、符号名、文件行号、指标变化和近期修改次数。
5. 扫描可信度摘要：分析文件数、语言、排除项、跳过项；点击进入扫描范围分类。
6. “交给 Agent 分析”：生成诊断上下文，明确显示将携带的位置、成因和验收目标。

### 结构复杂度

保留现有问题函数排行榜，同时增加：

- “本次新增 / 全部”筛选，默认本次新增。
- 函数 / 文件视角切换。
- 指标口径说明；现有 `complexity_signal` 显示名为“控制流信号”，不得标成标准圈复杂度。
- 每行展示相对上次的变化；没有基线时显示“首次扫描”。

### UI 一致性

保留六类现有检测。分类标题显示新增发现数和存量发现数。iced 专属检测必须注明“适用于 iced/Rust”；未识别到适用框架时显示“不适用”，不能显示“健康”。

### 扫描范围

展示：

- 扫描状态：完整、部分完成、失败。
- 已发现语言与文件数。
- 已分析文件数、排除文件数、跳过文件数。
- 每种排除或跳过原因及文件列表。
- 生效的忽略规则来源（内置、`.gitignore`、`.dozer/code-health.toml`）。
- 扫描耗时、报告版本和 Git 基准信息。

空状态拆成：尚未扫描、没有受支持代码、部分文件失败、扫描失败。不得把前三者合并成“没有可分析的 Rust 代码，或者还没有扫描过”。

## 数据模型

### 报告版本与扫描范围

`ProjectReport` 增加版本和扫描元数据：

```rust
pub struct ProjectReport {
    pub schema_version: u32,
    pub scan: ScanMetadata,
    pub git: Option<GitSnapshot>,
    pub findings: Vec<Finding>,
    // 现有字段在迁移期保留，后续由 findings 聚合替代。
}

pub struct ScanMetadata {
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub status: ScanStatus,
    pub discovered_files: usize,
    pub analyzed_files: usize,
    pub excluded_files: usize,
    pub skipped_files: Vec<SkippedFile>,
    pub languages: Vec<LanguageSummary>,
}

pub enum ScanStatus { Complete, Partial, Failed }

pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: SkipReason,
}

pub enum SkipReason {
    Ignored,
    UnsupportedLanguage,
    Generated,
    NonUtf8,
    ReadFailed,
    ParseFailed,
}
```

`schema_version` 首版取 `2`。读取旧 JSON 时通过 serde 默认值迁移为 v1 报告，并在 UI 标注“旧版报告，重新扫描可查看变化与范围”。

### 统一发现项

不同类别的发现使用统一外壳，便于计算差异和交给 agent：

```rust
pub struct Finding {
    pub id: String,
    pub rule_id: String,
    pub category: FindingCategory,
    pub severity: FindingSeverity,
    pub path: PathBuf,
    pub start_line: usize,
    pub symbol: Option<String>,
    pub title: String,
    pub evidence: FindingEvidence,
    pub applicability: Applicability,
}
```

`id` 必须稳定：使用 `rule_id + 规范化相对路径 + symbol + 规则相关结构签名` 计算，不包含当前指标值和行号。这样函数内新增几行不会被误判成“旧问题消失且新问题产生”。纯字面量发现没有符号时，使用规范化片段指纹和邻近语法节点。

### Git 快照与热点

```rust
pub struct GitSnapshot {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub dirty: bool,
}

pub struct Hotspot {
    pub finding_id: String,
    pub recent_commits: usize,
    pub metric_delta: Option<i64>,
    pub rank: HotspotRank,
    pub reasons: Vec<HotspotReason>,
}
```

热点排序不向用户暴露不可解释的小数总分。实现可使用离散排序元组：

1. 本次新增或恶化优先；
2. Critical 高于 Watch；
3. 近 30 天修改次数更高优先；
4. 指标增量更大优先；
5. 当前指标更高优先。

UI 将排序原因展开为文本，例如“本轮新增 31 行 · 近 30 天修改 19 次”。

### 历史存储

新增 `code_health_report_snapshots`，每次成功或部分成功扫描插入一行：

```sql
CREATE TABLE code_health_report_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    report_json TEXT NOT NULL,
    git_head TEXT,
    git_branch TEXT,
    git_dirty INTEGER NOT NULL,
    scanned_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_code_health_snapshots_project_time
ON code_health_report_snapshots(project_id, scanned_at_ms DESC);
```

每项目保留最近 30 份。现有 `code_health_reports` 在迁移期继续作为“最新报告”读取兼容入口；写入新快照后同时更新最新报告。差异比较采用“当前项目上一份可解析快照”，不要求 Git HEAD 不同。

## 扫描范围与忽略规则

文件遍历改用 `ignore` crate，遵守 `.gitignore`、`.ignore` 和隐藏目录规则，并叠加内置构建目录。项目可选配置：

```toml
# .dozer/code-health.toml
exclude = ["generated/**", "vendor/**"]
generated = ["**/*.generated.rs"]
```

受支持语言分两层：

- **统计支持：** 能识别语言并统计文件、代码行、注释与空行。
- **语义支持：** 能产生函数、复杂度或框架规则发现。

v2 必须至少完整支持 Rust；其他语言可以先只进入范围统计，但 UI 必须清楚标注“仅统计，未做结构分析”。

## Agent 诊断上下文

“交给 Agent 分析”只生成并传递以下只读上下文：

- 选中发现的位置和规则；
- 当前证据、相对上次变化、Git 热点原因；
- 建议验收目标，例如“控制流信号下降且无新增相关发现”；
- 扫描范围与已知局限。

用户点击后进入现有 agent 交互界面，是否修改以及如何修改由后续对话决定。本阶段不从代码健康模块直接执行编辑。

## 错误处理

- 单文件失败：继续扫描，状态为 `Partial`，记录路径和原因。
- Git 不可用：静态分析继续，热点退化为“变化 + 严重度”排序，UI 标注“无 Git 热点数据”。
- 快照保存失败：本次结果仍可展示，但必须显示“未保存，无法用于下次比较”。
- 旧报告反序列化失败：保留错误提示并允许重新扫描，不将其解释为从未扫描。
- 扫描任务取消或 panic：保留上一份结果，展示本次失败。

## 性能约束

- 10 万行 Rust 项目目标扫描时间：release 构建下不超过 15 秒；先记录基线再决定是否并行。
- 同一源文件只解析一次 AST，结构复杂度和 UI 规则共享根节点。
- Git 历史按文件批量查询，禁止逐发现项启动 git 子进程。
- 报告差异按稳定 ID 做哈希集合比较，时间复杂度为 O(n)。
- UI 默认仅渲染前 50 个热点，继续滚动或筛选时再展示更多。

## 测试与验收

### 自动测试

- `.gitignore`、项目配置、生成文件和读取失败的范围统计。
- 完整、部分、失败三种扫描状态。
- 稳定发现 ID 在行号变化和指标变化后保持不变。
- 新增、持续、恶化、改善和已解决差异计算。
- 无 Git、dirty 工作区和不同分支的快照元数据。
- 快照插入、最近两份查询、30 份清理和旧最新报告兼容读取。
- 热点排序规则及并列稳定性。
- 旧 v1 JSON 反序列化与重新扫描提示。

### 人工验收

1. 打开 Figma 对应页面核对布局：详情在左，分类导航在右，最右 Rail 不变。
2. 首次扫描显示“首次扫描”，不伪造增量数据。
3. 修改一个高频文件并新增复杂分支后再次扫描，该发现进入“本次新增/恶化”和优先处理区。
4. 修复发现后再次扫描，显示为已解决。
5. 制造不可读或无法解析文件，扫描仍完成并明确显示为部分完成。
6. 点击热点能打开对应文件与行；点击扫描可信度能进入完整范围列表。

## 发布切片

1. **可信扫描：** 报告版本、ignore 遍历、扫描范围与准确空状态。
2. **变化：** 历史快照、稳定发现 ID、相邻报告差异。
3. **热点：** Git 变动统计和优先处理排序。
4. **新 UI：** 总览与右侧分类导航、范围页、筛选和诊断上下文。

每个切片都必须可以独立发布；在历史与热点未完成前，总览不得用伪造变化值填充。

## 实现偏差记录（2026-09-21 实现时确认）

以下是与设计文档的差异，均为实现时确认的合理取舍，非规格变更：

1. **快照表冗余标量列。** `code_health_report_snapshots` 除 spec DDL 列的
   `schema_version/report_json/git_*/scanned_at_ms` 外，额外冗余存
   `total_loc/total_functions/critical_functions/overall_tier` 四列。理由：与
   `code_health_reports` 的"标量列 + 大文本列并存"哲学一致，`get`/`list`
   不需要每次反序列化整份 JSON 就能补全 `CodeHealthReportInfo` 的粗粒度字段。
2. **发现项路径相对化。** `Finding.path`/`functions`/`color_findings` 等所有
   路径统一存为相对项目根目录的规范化路径；`dozer-app` 在 `OpenLocation`
   边界用 `active_project_path()` 解析成绝对路径再开文件。
3. **Git 变动统计用单次批量命令。** `git log --since=30.days --name-only
   --relative`（`--relative` 让路径相对项目根，与扫描发现同口径）；Git 不可用
   时热点退化为"变化 + 严重度"排序，UI 标注无 Git 热点数据。
4. **性能基线用 ignored test 而非 bench。** workspace 无 benchmark 基础设施，
   按计划改为 `#[ignore]` 的 `scan_performance_baseline`；实测 10 万行 release
   扫描 1.53s（< 15s 目标），未引入按文件并行。
5. **`dozer-app` 无 lib target。** 计划的 `cargo test -p dozer-app --lib` 改为
   `cargo test -p dozer-app --bin dozer`（二进制内嵌单元测试）。
6. **UI 一致性"不适用"判定。** 以"是否语义分析了 Rust（`analyzed && files>0`）"
   为代理信号；未识别到 Rust/iced 时显示"不适用"而非"健康"。
7. **`交给 Agent 分析` 送输入区。** 诊断文本写入当前 agent 会话 PTY 输入（不带
   换行，不自动发送），用户仍需主动回车；无活动会话时为无操作。
