# 代码健康度面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增一个"代码健康度"面板：新 crate `dozer-codehealth` 用 ast-grep 对项目 Rust 代码做结构分析（函数/文件/项目三层复杂度指标），结果经 dozerd 落盘持久化，`dozer-app` 侧新增独立面板展示健康卡片 + 问题列表，点击问题可跳转到 Files 面板对应代码行。只做"预警 + 定位"，不做自动重构/发起 agent 任务。

**Architecture:** 数据分析（`dozer-codehealth`，纯函数库）→ 扫描结果经 UDS 协议存入 `dozerd` 的 `dozer.db`（新表 `code_health_reports`，仿 `bookmarks`/`session_summaries` 现有存储模式）→ `dozer-app` 新增 `extensions/codehealth/` 面板（仿 `extensions/usage/` 分层），手动触发扫描、打开面板读取上次落盘结果 → 问题列表点击项复用/扩展现有原生编辑器打开链路（`App::preview_open_path`）跳转定位。

**Tech Stack:** Rust, `ast-grep-core` + `ast-grep-language`（结构解析）、`rusqlite`（dozerd 持久化）、`iced` 0.14（面板 UI）。

**Spec:** `docs/superpowers/specs/2026-09-20-code-health-panel-design.md`

## Global Constraints

- 只做 Rust（一期不支持其他语言，语言检测直接过滤 `.rs` 扩展名）。
- `dozer-codehealth` crate 不依赖 `iced`/`dozer-core::protocol`/UDS，是纯函数库（spec「架构与数据流」第 1 节）。
- 复杂度/规模阈值一期全部写死为常量，不做配置面板。
- 扫描一期是手动触发（点按钮），面板打开不自动扫描，只读取上次落盘结果。
- 面板配色：健康态用 `byteui::theme::color::ColorTokens` 的 `green`(`#1AD585`)/`cyan`(`#47DEF0`)/`red`(`#FF6E6E`)；**禁止**用 `gold`（甲方动作专属色，CLAUDE.md 明确裁决）。
- 非代码/终端场景的文字一律用 `Font::default()`，不得用 `assets::fonts::code_font()`（等宽代码字体只给 code editor/pty 终端用，CLAUDE.md 关键裁决）。
- 中文渲染涉及自绘文本（canvas `fill_text` 等）一律用 `Shaping::Advanced`，不得用 `Shaping::Basic`（同上裁决）。
- 依赖只加 MIT/兼容协议的纯 Rust crate，不引入 Node/Python 运行时依赖。
- Rail 图标：`icons::IconKind::SquareActivity`，svg 取自 Lucide `square-activity`，格式对齐现有资产（无 `class` 属性、无许可证注释行，见 Task 9）。

---

### Task 1: 新建 `dozer-codehealth` crate 骨架

**Files:**
- Create: `crates/dozer-codehealth/Cargo.toml`
- Create: `crates/dozer-codehealth/src/lib.rs`

**Interfaces:**
- Produces: crate `dozer-codehealth`，供 Task 2/3 填充实现，供 Task 7 消费。

- [ ] **Step 1: 创建 crate 目录与 Cargo.toml**

`crates/dozer-codehealth/Cargo.toml`：

```toml
[package]
name = "dozer-codehealth"
version = "0.1.0"
edition.workspace = true
publish = false

[dependencies]
anyhow.workspace = true
```

- [ ] **Step 2: 创建空 lib.rs**

`crates/dozer-codehealth/src/lib.rs`：

```rust
//! Rust 代码结构分析：函数/文件/项目三层复杂度指标，供「代码健康度」
//! 面板使用（spec docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。
//! 纯函数库，不依赖 iced/UDS 协议。
```

- [ ] **Step 3: 确认 workspace 已识别新 crate**

`Cargo.toml`（仓库根）的 `[workspace] members = ["crates/*", "spike/*"]` 已经是通配符，新目录会被自动纳入，不需要手改。运行确认：

```bash
cargo metadata --no-deps --format-version 1 | grep -o '"dozer-codehealth"'
```

Expected: 输出 `"dozer-codehealth"`（至少一次）。

- [ ] **Step 4: 用 `cargo add` 引入 ast-grep 依赖（不手写版本号）**

```bash
cd crates/dozer-codehealth && cargo add ast-grep-core ast-grep-language
```

`spike/code-analysis/Cargo.toml` 已验证过这两个 crate 能正常解析（`ast-grep-core = "0.45.3"`、`ast-grep-language = "0.45.3"`），此处用 `cargo add` 走同样的 workspace 惯例，由 cargo 自行解析当前可用的兼容版本。

- [ ] **Step 5: 编译确认**

```bash
cargo build -p dozer-codehealth
```

Expected: 编译成功，无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-codehealth Cargo.toml Cargo.lock
git commit -m "feat(codehealth): 新建 dozer-codehealth crate 骨架"
```

---

### Task 2: 函数级复杂度指标

**Files:**
- Modify: `crates/dozer-codehealth/src/lib.rs`
- Create: `crates/dozer-codehealth/src/function_metric.rs`

**Interfaces:**
- Consumes: `ast_grep_core::Node`、`ast_grep_language::{LanguageExt, SupportLang}`（外部 crate，Task 1 已加依赖）。
- Produces（供 Task 3 使用）：
  - `pub struct FunctionMetric { pub name: String, pub file: PathBuf, pub start_line: usize, pub end_line: usize, pub loc: usize, pub complexity_signal: usize, pub severity: Severity }`
  - `pub enum Severity { Normal, Watch, Critical }`（`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`）
  - `pub fn severity_for(complexity_signal: usize) -> Severity`
  - `pub fn functions_in_source(src: &str, file: &Path) -> Vec<FunctionMetric>`

- [ ] **Step 1: 写失败测试——`severity_for` 三档边界**

`crates/dozer-codehealth/src/function_metric.rs`：

```rust
use ast_grep_core::Doc;
use ast_grep_language::{LanguageExt, SupportLang};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Normal,
    Watch,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionMetric {
    pub name: String,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub loc: usize,
    pub complexity_signal: usize,
    pub severity: Severity,
}

/// spec「函数级」判定表：`> 40` Critical，`16..=40` Watch，`<= 15` Normal。
pub fn severity_for(complexity_signal: usize) -> Severity {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_boundaries() {
        assert_eq!(severity_for(15), Severity::Normal);
        assert_eq!(severity_for(16), Severity::Watch);
        assert_eq!(severity_for(40), Severity::Watch);
        assert_eq!(severity_for(41), Severity::Critical);
    }
}
```

（`todo!()` 只出现在这一步的失败测试阶段，Step 3 会立刻替换成真实实现——不是计划的最终状态。）

- [ ] **Step 2: 运行确认失败**

```bash
cargo test -p dozer-codehealth severity_boundaries
```

Expected: FAIL（panic `not yet implemented`）。

- [ ] **Step 3: 实现 `severity_for`**

```rust
pub fn severity_for(complexity_signal: usize) -> Severity {
    if complexity_signal > 40 {
        Severity::Critical
    } else if complexity_signal >= 16 {
        Severity::Watch
    } else {
        Severity::Normal
    }
}
```

- [ ] **Step 4: 运行确认通过**

```bash
cargo test -p dozer-codehealth severity_boundaries
```

Expected: PASS。

- [ ] **Step 5: 写失败测试——`functions_in_source` 解析单个函数**

在同一文件追加：

```rust
    #[test]
    fn functions_in_source_extracts_name_and_loc() {
        let src = "fn foo() {\n    let x = 1;\n    x\n}\n";
        let metrics = functions_in_source(src, Path::new("a.rs"));
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "foo");
        assert_eq!(metrics[0].file, Path::new("a.rs"));
        assert_eq!(metrics[0].start_line, 1);
        assert_eq!(metrics[0].end_line, 4);
        assert_eq!(metrics[0].loc, 4);
        assert_eq!(metrics[0].complexity_signal, 0);
        assert_eq!(metrics[0].severity, Severity::Normal);
    }

    #[test]
    fn functions_in_source_counts_control_flow_nodes() {
        let src = "fn bar(n: i32) -> i32 {\n    if n > 0 {\n        for i in 0..n {\n            let _ = i;\n        }\n    }\n    match n {\n        0 => 0,\n        _ => n,\n    }\n}\n";
        let metrics = functions_in_source(src, Path::new("b.rs"));
        assert_eq!(metrics.len(), 1);
        // if(1) + for(1) + match(1) = 3
        assert_eq!(metrics[0].complexity_signal, 3);
    }

    #[test]
    fn functions_in_source_handles_multiple_functions() {
        let src = "fn one() {}\nfn two() {}\n";
        let metrics = functions_in_source(src, Path::new("c.rs"));
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "one");
        assert_eq!(metrics[1].name, "two");
    }

    #[test]
    fn functions_in_source_empty_file_returns_empty() {
        assert!(functions_in_source("", Path::new("empty.rs")).is_empty());
    }
```

- [ ] **Step 6: 运行确认失败（`functions_in_source` 还不存在）**

```bash
cargo test -p dozer-codehealth functions_in_source
```

Expected: FAIL（编译错误，函数未定义）。

- [ ] **Step 7: 实现 `functions_in_source`**

同 `spike/code-analysis/src/main.rs` 已验证过的解析方式（`SupportLang::Rust.ast_grep(src)` → `root().dfs()` 过滤 `function_item` → `dfs()` 数控制流节点）：

```rust
const CONTROL_FLOW_KINDS: &[&str] = &[
    "if_expression",
    "match_expression",
    "for_expression",
    "while_expression",
    "loop_expression",
    "closure_expression",
];

fn complexity_signal_of<D: Doc>(node: &ast_grep_core::Node<'_, D>) -> usize {
    node.dfs()
        .filter(|n| CONTROL_FLOW_KINDS.contains(&n.kind().as_ref()))
        .count()
}

pub fn functions_in_source(src: &str, file: &Path) -> Vec<FunctionMetric> {
    let root = SupportLang::Rust.ast_grep(src);
    root.root()
        .dfs()
        .filter(|n| n.kind() == "function_item")
        .map(|f| {
            let name = f
                .field("name")
                .map(|n| n.text().to_string())
                .unwrap_or_else(|| "<anonymous>".to_string());
            let start_line = f.start_pos().line() + 1;
            let end_line = f.end_pos().line() + 1;
            let complexity_signal = complexity_signal_of(&f);
            FunctionMetric {
                name,
                file: file.to_path_buf(),
                start_line,
                end_line,
                loc: end_line - start_line + 1,
                complexity_signal,
                severity: severity_for(complexity_signal),
            }
        })
        .collect()
}
```

- [ ] **Step 8: 运行确认全部通过**

```bash
cargo test -p dozer-codehealth
```

Expected: PASS（本文件全部测试）。

- [ ] **Step 9: 挂到 lib.rs**

`crates/dozer-codehealth/src/lib.rs` 追加：

```rust
mod function_metric;
pub use function_metric::{severity_for, functions_in_source, FunctionMetric, Severity};
```

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-codehealth
git commit -m "feat(codehealth): 函数级复杂度指标解析"
```

---

### Task 3: 文件级 + 项目级聚合、`scan_project` 入口

**Files:**
- Create: `crates/dozer-codehealth/src/report.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`

**Interfaces:**
- Consumes: `FunctionMetric`、`Severity`（Task 2）。
- Produces（供 Task 7 消费）：
  - `pub struct FileMetric { pub path: PathBuf, pub loc: usize, pub critical_functions: usize, pub watch_functions: usize, pub flagged: bool }`
  - `pub enum HealthTier { Healthy, Watch, Critical }`（`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`，且 `impl Ord`/`PartialOrd` 使 `Critical > Watch > Healthy`，供 `overall_tier` 的 `max` 规则用）
  - `pub struct ProjectReport { pub total_loc: usize, pub total_functions: usize, pub critical_functions: usize, pub scale_tier: HealthTier, pub density_tier: HealthTier, pub overall_tier: HealthTier, pub functions: Vec<FunctionMetric> }`
  - `pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport>`（顶层入口，Task 7 直接调用）

- [ ] **Step 1: 写失败测试——`HealthTier` 排序**

`crates/dozer-codehealth/src/report.rs`：

```rust
use crate::function_metric::{severity_for, FunctionMetric, Severity};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthTier {
    Healthy,
    Watch,
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_tier_ordered_critical_highest() {
        assert!(HealthTier::Critical > HealthTier::Watch);
        assert!(HealthTier::Watch > HealthTier::Healthy);
        assert_eq!(
            HealthTier::Healthy.max(HealthTier::Critical),
            HealthTier::Critical
        );
    }
}
```

（枚举按声明顺序派生 `Ord`——`Healthy` 声明在前即最小，已满足题意，不用手写 `impl Ord`。）

- [ ] **Step 2: 运行确认通过（这步不需要失败，派生的 `Ord` 直接正确）**

```bash
cargo test -p dozer-codehealth health_tier_ordered_critical_highest
```

Expected: PASS。

- [ ] **Step 3: 写失败测试——文件级聚合 `flagged` 判定**

追加：

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct FileMetric {
    pub path: PathBuf,
    pub loc: usize,
    pub critical_functions: usize,
    pub watch_functions: usize,
    pub flagged: bool,
}

/// spec「文件级」：含 ≥1 个 Critical 函数，或文件总行数 > 1000 → flagged。
pub fn file_metric(path: &Path, file_loc: usize, functions: &[FunctionMetric]) -> FileMetric {
    todo!()
}

#[cfg(test)]
mod tests {
    // ...(上面已有的测试保留)...

    #[test]
    fn file_metric_flags_on_critical_function() {
        let f = FunctionMetric {
            name: "x".into(),
            file: PathBuf::from("a.rs"),
            start_line: 1,
            end_line: 2,
            loc: 2,
            complexity_signal: 41,
            severity: severity_for(41),
        };
        let m = file_metric(Path::new("a.rs"), 100, &[f]);
        assert_eq!(m.critical_functions, 1);
        assert_eq!(m.watch_functions, 0);
        assert!(m.flagged);
    }

    #[test]
    fn file_metric_flags_on_loc_over_1000_even_without_critical_function() {
        let m = file_metric(Path::new("a.rs"), 1001, &[]);
        assert!(m.flagged);
    }

    #[test]
    fn file_metric_not_flagged_when_small_and_no_critical() {
        let m = file_metric(Path::new("a.rs"), 1000, &[]);
        assert!(!m.flagged);
    }
}
```

- [ ] **Step 4: 运行确认失败**

```bash
cargo test -p dozer-codehealth file_metric
```

Expected: FAIL（`todo!()` panic）。

- [ ] **Step 5: 实现 `file_metric`**

```rust
pub fn file_metric(path: &Path, file_loc: usize, functions: &[FunctionMetric]) -> FileMetric {
    let critical_functions = functions
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let watch_functions = functions
        .iter()
        .filter(|f| f.severity == Severity::Watch)
        .count();
    FileMetric {
        path: path.to_path_buf(),
        loc: file_loc,
        critical_functions,
        watch_functions,
        flagged: critical_functions > 0 || file_loc > 1000,
    }
}
```

- [ ] **Step 6: 运行确认通过**

```bash
cargo test -p dozer-codehealth file_metric
```

Expected: PASS。

- [ ] **Step 7: 写失败测试——项目级分档与 `overall_tier` 的 max 规则**

追加：

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectReport {
    pub total_loc: usize,
    pub total_functions: usize,
    pub critical_functions: usize,
    pub scale_tier: HealthTier,
    pub density_tier: HealthTier,
    pub overall_tier: HealthTier,
    pub functions: Vec<FunctionMetric>,
}

/// spec「规模分档」：`< 10_000` Healthy，`10_000..=20_000` Watch，`> 20_000` Critical。
pub fn scale_tier(total_loc: usize) -> HealthTier {
    todo!()
}

/// spec「密度分档」：`< 2%` Healthy，`2%..=5%` Watch，`> 5%` Critical。
/// `total_functions == 0` 视为 Healthy（没有函数就没有问题函数可言）。
pub fn density_tier(critical_functions: usize, total_functions: usize) -> HealthTier {
    todo!()
}

#[cfg(test)]
mod tests {
    // ...(上面已有测试保留)...

    #[test]
    fn scale_tier_boundaries() {
        assert_eq!(scale_tier(9_999), HealthTier::Healthy);
        assert_eq!(scale_tier(10_000), HealthTier::Watch);
        assert_eq!(scale_tier(20_000), HealthTier::Watch);
        assert_eq!(scale_tier(20_001), HealthTier::Critical);
    }

    #[test]
    fn density_tier_boundaries() {
        assert_eq!(density_tier(1, 100), HealthTier::Healthy); // 1%
        assert_eq!(density_tier(2, 100), HealthTier::Watch); // 2%
        assert_eq!(density_tier(5, 100), HealthTier::Watch); // 5%
        assert_eq!(density_tier(6, 100), HealthTier::Critical); // 6%
    }

    #[test]
    fn density_tier_zero_functions_is_healthy() {
        assert_eq!(density_tier(0, 0), HealthTier::Healthy);
    }
}
```

- [ ] **Step 8: 运行确认失败**

```bash
cargo test -p dozer-codehealth scale_tier_boundaries density_tier
```

Expected: FAIL。

- [ ] **Step 9: 实现两个分档函数**

```rust
pub fn scale_tier(total_loc: usize) -> HealthTier {
    if total_loc > 20_000 {
        HealthTier::Critical
    } else if total_loc >= 10_000 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

pub fn density_tier(critical_functions: usize, total_functions: usize) -> HealthTier {
    if total_functions == 0 {
        return HealthTier::Healthy;
    }
    let pct = critical_functions as f64 / total_functions as f64 * 100.0;
    if pct > 5.0 {
        HealthTier::Critical
    } else if pct >= 2.0 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}
```

- [ ] **Step 10: 运行确认通过**

```bash
cargo test -p dozer-codehealth scale_tier_boundaries density_tier
```

Expected: PASS。

- [ ] **Step 11: 写失败测试——`scan_project` 端到端（用临时目录构造真实 `.rs` 文件）**

追加：

```rust
#[cfg(test)]
mod scan_tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_project_empty_dir_returns_all_zero() {
        let dir = tempfile::tempdir().unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_loc, 0);
        assert_eq!(report.total_functions, 0);
        assert_eq!(report.critical_functions, 0);
        assert_eq!(report.overall_tier, HealthTier::Healthy);
        assert!(report.functions.is_empty());
    }

    #[test]
    fn scan_project_skips_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/generated.rs"), "fn ignored() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 0);
    }

    #[test]
    fn scan_project_skips_unparseable_file_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("good.rs"), "fn ok() {}").unwrap();
        // 非法 UTF-8 字节序列——`fs::read_to_string` 会报错，该文件应被跳过
        // 而不是让整体扫描失败。
        fs::write(dir.path().join("bad.rs"), [0xff, 0xfe, 0x00]).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 1);
    }

    #[test]
    fn scan_project_aggregates_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        fs::write(dir.path().join("c.rs"), "fn c() {}\n").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 3);
    }

    #[test]
    fn scan_project_overall_tier_takes_worse_of_scale_and_density() {
        let dir = tempfile::tempdir().unwrap();
        // 造一个复杂度信号 41(Critical)的函数,规模很小(远低于 1 万行),
        // scale_tier 应为 Healthy、density_tier 应为 Critical(1/1 = 100%),
        // overall_tier 取二者较严重者 = Critical。
        let body = "if a {}".repeat(41 - 1); // 40 个 if 之外再手写一个,凑够 41
        let src = format!("fn heavy() {{\n    if a {{}}\n{body}\n}}\n");
        fs::write(dir.path().join("heavy.rs"), src).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.scale_tier, HealthTier::Healthy);
        assert_eq!(report.density_tier, HealthTier::Critical);
        assert_eq!(report.overall_tier, HealthTier::Critical);
    }
}
```

- [ ] **Step 12: 运行确认失败**

```bash
cargo test -p dozer-codehealth scan_project
```

Expected: FAIL（`scan_project` 未定义 + 缺 `tempfile` dev-dependency 编译错误）。

- [ ] **Step 13: 加 `tempfile` dev-dependency**

```bash
cd crates/dozer-codehealth && cargo add --dev tempfile
```

- [ ] **Step 14: 实现 `scan_project`（含递归收集 `.rs` 文件、跳过 `target/`、单文件失败跳过）**

```rust
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport> {
    let mut files = Vec::new();
    if root.is_dir() {
        collect_rs_files(root, &mut files)?;
    }

    let mut all_functions = Vec::new();
    let mut total_loc = 0usize;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            // 单文件读不到/非 UTF-8 → 跳过,不中断整体扫描(见 spec「错误处理」)。
            continue;
        };
        total_loc += src.lines().count();
        all_functions.extend(crate::function_metric::functions_in_source(&src, path));
    }

    let total_functions = all_functions.len();
    let critical_functions = all_functions
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let scale = scale_tier(total_loc);
    let density = density_tier(critical_functions, total_functions);
    let mut functions = all_functions;
    functions.sort_by(|a, b| b.severity_rank().cmp(&a.severity_rank()));

    Ok(ProjectReport {
        total_loc,
        total_functions,
        critical_functions,
        scale_tier: scale,
        density_tier: density,
        overall_tier: scale.max(density),
        functions,
    })
}
```

`functions.sort_by` 用到的 `severity_rank()` 还不存在，下一步补在 `FunctionMetric` 上（`Severity` 本身没有派生 `Ord`，用一个小助手方法避免给 `Severity` 加 `Ord` derive 却打乱它在 Task 2 里已经固定的字段顺序语义）：

回到 `crates/dozer-codehealth/src/function_metric.rs`，给 `FunctionMetric` 加：

```rust
impl FunctionMetric {
    /// 供 `ProjectReport.functions` 按严重度降序排列用，不对外暴露 `Severity`
    /// 的 `Ord`——`Critical` 应排最前。
    fn severity_rank(&self) -> u8 {
        match self.severity {
            Severity::Critical => 2,
            Severity::Watch => 1,
            Severity::Normal => 0,
        }
    }
}
```

（`severity_rank` 是私有方法但 `report.rs` 要调用——两个文件同属 `dozer-codehealth` crate 内部,改成 `pub(crate) fn severity_rank`。）

- [ ] **Step 15: 运行确认全部通过**

```bash
cargo test -p dozer-codehealth
```

Expected: PASS（全部测试，包括 Task 2 的）。

- [ ] **Step 16: 挂到 lib.rs 并暴露公共 API**

`crates/dozer-codehealth/src/lib.rs` 最终形态：

```rust
//! Rust 代码结构分析：函数/文件/项目三层复杂度指标，供「代码健康度」
//! 面板使用（spec docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。
//! 纯函数库，不依赖 iced/UDS 协议。

mod function_metric;
mod report;

pub use function_metric::{functions_in_source, severity_for, FunctionMetric, Severity};
pub use report::{density_tier, file_metric, scale_tier, scan_project, FileMetric, HealthTier, ProjectReport};
```

- [ ] **Step 17: Commit**

```bash
git add crates/dozer-codehealth
git commit -m "feat(codehealth): 文件/项目级聚合与 scan_project 入口"
```

---

### Task 4: `dozer-core::protocol` 新增持久化消息

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces（供 Task 5/6 使用）：
  - `pub struct CodeHealthReportInfo { pub total_loc: u64, pub total_functions: u64, pub critical_functions: u64, pub overall_tier: String, pub report_json: String, pub scanned_at_ms: u64 }`
  - `Request::SaveCodeHealthReport { project_id: i64, report_json: String, total_loc: u64, total_functions: u64, critical_functions: u64, overall_tier: String }`
  - `Request::GetCodeHealthReport { project_id: i64 }`
  - `Reply::CodeHealthReport { report: Option<CodeHealthReportInfo> }`
  - （保存成功复用现有 `Reply::Ok`，不新增专门的 ack 变体——同 `RemoveBookmark` 的既有做法。）

- [ ] **Step 1: 写失败测试——序列化往返**

在 `crates/dozer-core/src/protocol.rs` 的 `#[cfg(test)] mod tests` 里追加（紧邻现有 `Request::AddBookmark`/`Request::ListBookmarks` 序列化测试旁边）：

```rust
    #[test]
    fn save_code_health_report_serializes_with_type_tag() {
        let req = Request::SaveCodeHealthReport {
            project_id: 1,
            report_json: "{}".into(),
            total_loc: 100,
            total_functions: 10,
            critical_functions: 1,
            overall_tier: "watch".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"save_code_health_report\""));
    }

    #[test]
    fn get_code_health_report_round_trips() {
        let req = Request::GetCodeHealthReport { project_id: 7 };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn code_health_report_reply_round_trips_with_none() {
        let reply = Reply::CodeHealthReport { report: None };
        let json = serde_json::to_string(&reply).unwrap();
        let back: Reply = serde_json::from_str(&json).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn code_health_report_info_round_trips() {
        let info = CodeHealthReportInfo {
            total_loc: 100,
            total_functions: 10,
            critical_functions: 1,
            overall_tier: "critical".into(),
            report_json: "{\"functions\":[]}".into(),
            scanned_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: CodeHealthReportInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }
```

- [ ] **Step 2: 运行确认失败**

```bash
cargo test -p dozer-core code_health
```

Expected: FAIL（类型/变体未定义，编译错误）。

- [ ] **Step 3: 加 `CodeHealthReportInfo` 类型**

紧邻 `BookmarkInfo`（`protocol.rs:194` 附近）之后加：

```rust
/// 一次代码健康度扫描的落盘结果。`report_json` 是
/// `dozer_codehealth::ProjectReport` 的完整序列化（含 `functions` 明细列表），
/// 顶部几个标量字段冗余存一份是为了 dozerd 侧不需要反序列化整个 JSON
/// 就能回答"这个项目健康度是什么档位"这类粗粒度查询（目前没有这类查询，
/// 但同 `session_summaries` 表"标量列 + 大文本列"并存的既有设计一致，
/// 不额外增加复杂度）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeHealthReportInfo {
    pub total_loc: u64,
    pub total_functions: u64,
    pub critical_functions: u64,
    pub overall_tier: String,
    pub report_json: String,
    pub scanned_at_ms: u64,
}
```

- [ ] **Step 4: 加 `Request` 两个变体**

在 `enum Request`（`protocol.rs:306` 起）内，紧邻 `Request::ListBookmarks` 之后加：

```rust
    SaveCodeHealthReport {
        project_id: i64,
        report_json: String,
        total_loc: u64,
        total_functions: u64,
        critical_functions: u64,
        overall_tier: String,
    },
    GetCodeHealthReport {
        project_id: i64,
    },
```

- [ ] **Step 5: 加 `Reply::CodeHealthReport` 变体**

在 `enum Reply`（`protocol.rs:571` 起）内，紧邻 `Reply::Bookmarks` 之后加：

```rust
    /// `GetCodeHealthReport` 应答。`None` = 这个项目还没扫描过。
    CodeHealthReport {
        report: Option<CodeHealthReportInfo>,
    },
```

- [ ] **Step 6: 运行确认全部通过**

```bash
cargo test -p dozer-core
```

Expected: PASS（全部测试，含既有的）。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-core
git commit -m "feat(protocol): 新增代码健康度报告的存取消息"
```

---

### Task 5: dozerd 持久化 store

**Files:**
- Create: `crates/dozerd/src/code_health.rs`
- Modify: `crates/dozerd/src/lib.rs`
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::{CodeHealthReportInfo, Request, Reply}`（Task 4）。
- Produces: `pub struct CodeHealthStore { .. }`，`pub fn new(path: &Path) -> Result<Self>`，`pub fn save(&self, project_id: i64, info: &CodeHealthReportInfo) -> Result<()>`，`pub fn get(&self, project_id: i64) -> Result<Option<CodeHealthReportInfo>>`。

- [ ] **Step 1: 写 `CodeHealthStore`（表结构仿 `bookmarks.rs`/`session_summary.rs`，一个项目一行，`INSERT OR REPLACE` 做 upsert）**

`crates/dozerd/src/code_health.rs`：

```rust
//! 代码健康度报告存储：rusqlite 单表，主键 project_id（一个项目只保留
//! 最新一次扫描结果，历史不保留——同 spec「架构与数据流」的落盘持久化
//! 需求，只要"上次结果"，不要历史趋势）。
//! `scanned_at_ms` 由本 store 内部用 `now_ms()` 生成，调用方不传（同
//! `bookmarks.rs::add` 的既有约定：时间戳由服务端权威生成）。

use anyhow::{Context, Result};
use dozer_core::protocol::CodeHealthReportInfo;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub struct CodeHealthStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl CodeHealthStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS code_health_reports (
                project_id INTEGER PRIMARY KEY,
                total_loc INTEGER NOT NULL,
                total_functions INTEGER NOT NULL,
                critical_functions INTEGER NOT NULL,
                overall_tier TEXT NOT NULL,
                report_json TEXT NOT NULL,
                scanned_at_ms INTEGER NOT NULL
             );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn save(&self, project_id: i64, info: &CodeHealthReportInfo) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let ts = now_ms() as i64;
        conn.execute(
            "INSERT INTO code_health_reports
                (project_id, total_loc, total_functions, critical_functions,
                 overall_tier, report_json, scanned_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(project_id) DO UPDATE SET
                total_loc = excluded.total_loc,
                total_functions = excluded.total_functions,
                critical_functions = excluded.critical_functions,
                overall_tier = excluded.overall_tier,
                report_json = excluded.report_json,
                scanned_at_ms = excluded.scanned_at_ms",
            rusqlite::params![
                project_id,
                info.total_loc as i64,
                info.total_functions as i64,
                info.critical_functions as i64,
                info.overall_tier,
                info.report_json,
                ts,
            ],
        )
        .context("写入代码健康度报告")?;
        Ok(())
    }

    pub fn get(&self, project_id: i64) -> Result<Option<CodeHealthReportInfo>> {
        let conn = self.conn.lock().expect("db lock");
        conn.query_row(
            "SELECT total_loc, total_functions, critical_functions, overall_tier,
                    report_json, scanned_at_ms
             FROM code_health_reports WHERE project_id = ?1",
            [project_id],
            |row| {
                Ok(CodeHealthReportInfo {
                    total_loc: row.get::<_, i64>(0)? as u64,
                    total_functions: row.get::<_, i64>(1)? as u64,
                    critical_functions: row.get::<_, i64>(2)? as u64,
                    overall_tier: row.get(3)?,
                    report_json: row.get(4)?,
                    scanned_at_ms: row.get::<_, i64>(5)? as u64,
                })
            },
        )
        .optional()
        .context("查询代码健康度报告")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(loc: u64) -> CodeHealthReportInfo {
        CodeHealthReportInfo {
            total_loc: loc,
            total_functions: 10,
            critical_functions: 1,
            overall_tier: "watch".into(),
            report_json: "{}".into(),
            scanned_at_ms: 0, // 忽略，store 会覆盖成自己的 now_ms()
        }
    }

    #[test]
    fn get_missing_project_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        assert_eq!(store.get(1).unwrap(), None);
    }

    #[test]
    fn save_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        let got = store.get(1).unwrap().expect("应有数据");
        assert_eq!(got.total_loc, 100);
        assert_eq!(got.total_functions, 10);
        assert_eq!(got.critical_functions, 1);
        assert_eq!(got.overall_tier, "watch");
        assert!(got.scanned_at_ms > 0);
    }

    #[test]
    fn save_twice_upserts_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        store.save(1, &info(200)).unwrap();
        let got = store.get(1).unwrap().expect("应有数据");
        assert_eq!(got.total_loc, 200);
    }

    #[test]
    fn different_projects_independent() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        store.save(2, &info(999)).unwrap();
        assert_eq!(store.get(1).unwrap().unwrap().total_loc, 100);
        assert_eq!(store.get(2).unwrap().unwrap().total_loc, 999);
    }
}
```

`OptionalExtension` 需要确认 `rusqlite` 已在 `crates/dozerd/Cargo.toml` 启用了该扩展 trait 所在的默认 feature——`bookmarks.rs` 已经用了普通 `query_row`（非 optional 场景），先跑一次 Step 2 确认编译，若 `OptionalExtension` 未被导出再回来检查 `rusqlite` 版本/feature。

- [ ] **Step 2: 运行测试（这几个测试不需要先看到失败——本文件是新文件，实现和测试同批写，直接跑确认通过）**

```bash
cargo test -p dozerd code_health
```

Expected: PASS，若报 `OptionalExtension` 相关的 trait-not-in-scope 编译错误，检查 `crates/dozerd/Cargo.toml` 里 `rusqlite` 的声明方式（对照 `bookmarks.rs`/`todo.rs` 已经能用的 import 路径）。

- [ ] **Step 3: 挂进 `dozerd/src/lib.rs`**

`crates/dozerd/src/lib.rs` 加一行（紧邻 `pub mod bookmarks;`）：

```rust
pub mod code_health;
```

- [ ] **Step 4: 在 `main.rs` 构造 store**

`crates/dozerd/src/main.rs`，紧邻 `bookmarks` 构造（约第 82-84 行）之后加：

```rust
    let code_health = Arc::new(dozerd::code_health::CodeHealthStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

- [ ] **Step 5: 把 `code_health` 传进 server 启动调用**

`main.rs` 里 `bookmarks` 变量后续被塞进某个结构体字面量（约第 117 行 `bookmarks,`）——同样位置加一行 `code_health,`。

- [ ] **Step 6: server.rs 接住新字段（仿 `bookmarks` 逐处添加）**

`crates/dozerd/src/server.rs`，以下 4 处，每处紧邻已有的 `bookmarks` 那一行加对应的 `code_health` 行：

- 约第 75 行：`bookmarks: Arc<crate::bookmarks::BookmarkStore>,` 旁加
  `code_health: Arc<crate::code_health::CodeHealthStore>,`
- 约第 116/132 行：`let bookmarks = bookmarks.clone();` / 结构体字面量里的 `bookmarks,` 旁加对应的 `code_health` 克隆与传递
- 约第 401 行：同第 75 行，另一个结构体定义处
- 约第 1031/1042 行：测试辅助构造处，同上补齐

- [ ] **Step 7: server.rs 加请求分发**

在 `Request::ListBookmarks { .. } => { .. }` 分支（约第 591 行）之后加：

```rust
                        Request::SaveCodeHealthReport {
                            project_id,
                            report_json,
                            total_loc,
                            total_functions,
                            critical_functions,
                            overall_tier,
                        } => {
                            let info = dozer_core::protocol::CodeHealthReportInfo {
                                total_loc,
                                total_functions,
                                critical_functions,
                                overall_tier,
                                report_json,
                                scanned_at_ms: 0, // store 内部会用自己的 now_ms() 覆盖
                            };
                            match code_health.save(project_id, &info) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error {
                                    message: format!("保存代码健康度报告失败: {e}"),
                                },
                            }
                        }
                        Request::GetCodeHealthReport { project_id } => {
                            match code_health.get(project_id) {
                                Ok(report) => Reply::CodeHealthReport { report },
                                Err(e) => Reply::Error {
                                    message: format!("查询代码健康度报告失败: {e}"),
                                },
                            }
                        }
```

- [ ] **Step 8: 编译确认**

```bash
cargo build -p dozerd
```

Expected: 编译成功。若报"字段未初始化"之类的错误，说明 Step 6 有遗漏的结构体字面量，回去用编译器报错定位补全（这类"新增一个 `Arc<Store>` 字段、跟着所有既有 `bookmarks` 出现的地方补一行"的改动，编译器会把每个遗漏点都报出来，逐条修）。

- [ ] **Step 9: 运行全部 dozerd 测试**

```bash
cargo test -p dozerd
```

Expected: PASS。

- [ ] **Step 10: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): 代码健康度报告持久化 store"
```

---

### Task 6: dozer-client 方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes: `Request::SaveCodeHealthReport`/`Request::GetCodeHealthReport`/`Reply::CodeHealthReport`/`Reply::Ok`（Task 4）。
- Produces（供 Task 7 使用）：
  - `pub async fn save_code_health_report(&self, project_id: i64, report_json: String, total_loc: u64, total_functions: u64, critical_functions: u64, overall_tier: String) -> Result<()>`
  - `pub async fn get_code_health_report(&self, project_id: i64) -> Result<Option<CodeHealthReportInfo>>`

- [ ] **Step 1: 确认 import 列表已经带 `CodeHealthReportInfo`**

`crates/dozer-client/src/lib.rs` 顶部 `use dozer_core::protocol::{ .. }` 列表（约第 5 行起，`BookmarkInfo` 也在这里）里加入 `CodeHealthReportInfo`。

- [ ] **Step 2: 加两个方法（紧邻 `list_bookmarks`，约第 235 行之后）**

```rust
    pub async fn save_code_health_report(
        &self,
        project_id: i64,
        report_json: String,
        total_loc: u64,
        total_functions: u64,
        critical_functions: u64,
        overall_tier: String,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::SaveCodeHealthReport {
                project_id,
                report_json,
                total_loc,
                total_functions,
                critical_functions,
                overall_tier,
            })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => Err(anyhow::anyhow!("意外的回复类型: {other:?}")),
        }
    }

    pub async fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> Result<Option<CodeHealthReportInfo>> {
        match self
            .roundtrip(&Request::GetCodeHealthReport { project_id })
            .await?
        {
            Reply::CodeHealthReport { report } => Ok(report),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => Err(anyhow::anyhow!("意外的回复类型: {other:?}")),
        }
    }
```

核对相邻既有方法（如 `remove_bookmark`）实际怎么处理 `Reply::Error`/兜底分支——若既有惯例用了别的错误构造方式（比如不含 `anyhow::anyhow!("意外的回复类型...")` 这个兜底臂，因为 `roundtrip` 本身可能已经把非预期 Reply 处理成 Err 了），照抄既有惯例而不是这里写的样例，保持全文件风格一致。

- [ ] **Step 3: 编译确认**

```bash
cargo build -p dozer-client
```

Expected: 编译成功。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-client
git commit -m "feat(client): 代码健康度报告存取方法"
```

---

### Task 7: `dozer-app` 扩展数据层——`extensions/codehealth/`

**Files:**
- Create: `crates/dozer-app/src/extensions/codehealth/mod.rs`
- Create: `crates/dozer-app/src/extensions/codehealth/aggregate.rs`

**Interfaces:**
- Consumes: `dozer_codehealth::{scan_project, ProjectReport}`（Task 3），`Client::{save_code_health_report, get_code_health_report}`（Task 6）。
- Produces（供 Task 9 使用）：
  - `pub struct WorkspaceState { .. }` 带 `pub fn report(&self) -> Option<&ProjectReport>`、`pub fn scanned_at_ms(&self) -> Option<u64>`、`pub fn scanning(&self) -> bool`、`pub fn scan_error(&self) -> Option<&str>`
  - `pub enum Message { Loaded(i64, Option<ProjectReport>, u64), Scanned(i64, Result<ProjectReport, String>), ScanRequested }`
  - `pub fn update(ws_state: &mut WorkspaceState, msg: Message)`
  - `pub fn spawn_load_cached(project_id: i64, client: &dozer_client::Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`
  - `pub fn spawn_scan(project_id: i64, project_path: PathBuf, client: &dozer_client::Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`

- [ ] **Step 1: 写 `WorkspaceState` + `Message` + `update`（`mod.rs`）**

```rust
//! 代码健康度面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。仿
//! `extensions::usage` 的分层：`aggregate.rs` 做异步扫描/加载，`mod.rs`
//! 挂 `WorkspaceState`，`view.rs`（Task 8）渲染。

mod aggregate;
pub(crate) use aggregate::*;

use dozer_codehealth::ProjectReport;

#[derive(Default)]
pub struct WorkspaceState {
    report: Option<ProjectReport>,
    scanned_at_ms: Option<u64>,
    scanning: bool,
    /// 上一次 `Message::Scanned(_, Err(_))` 的错误文案，成功扫描或重新点
    /// "扫描"都会清掉（spec「错误处理」："扫描失败，请重试"，保留上一次
    /// 成功结果的同时要能看到这次失败了）。
    scan_error: Option<String>,
}

impl WorkspaceState {
    pub fn report(&self) -> Option<&ProjectReport> {
        self.report.as_ref()
    }

    pub fn scanned_at_ms(&self) -> Option<u64> {
        self.scanned_at_ms
    }

    pub fn scanning(&self) -> bool {
        self.scanning
    }

    pub fn scan_error(&self) -> Option<&str> {
        self.scan_error.as_deref()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 面板打开时读落盘缓存的结果。`Option` 为 `None` 代表这个项目还没
    /// 扫描过。
    Loaded(i64, Option<ProjectReport>, Option<u64>),
    /// 手动扫描结果回灌。
    Scanned(i64, Result<ProjectReport, String>),
    /// 点"扫描"按钮。
    ScanRequested,
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_project_id, report, scanned_at_ms) => {
            ws_state.report = report;
            ws_state.scanned_at_ms = scanned_at_ms;
        }
        Message::Scanned(_project_id, Ok(report)) => {
            ws_state.scanning = false;
            ws_state.scan_error = None;
            ws_state.report = Some(report);
            ws_state.scanned_at_ms = Some(now_ms());
        }
        Message::Scanned(_project_id, Err(e)) => {
            ws_state.scanning = false;
            ws_state.scan_error = Some(e);
            // 保留上一次成功结果不被覆盖（见 spec「错误处理」）：不touch
            // ws_state.report/scanned_at_ms。
        }
        Message::ScanRequested => {
            ws_state.scanning = true;
            ws_state.scan_error = None;
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::HealthTier;

    fn sample_report() -> ProjectReport {
        ProjectReport {
            total_loc: 100,
            total_functions: 5,
            critical_functions: 1,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Watch,
            overall_tier: HealthTier::Watch,
            functions: vec![],
        }
    }

    #[test]
    fn loaded_populates_report_and_timestamp() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Loaded(1, Some(sample_report()), Some(123)));
        assert!(ws.report().is_some());
        assert_eq!(ws.scanned_at_ms(), Some(123));
    }

    #[test]
    fn loaded_none_means_never_scanned() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Loaded(1, None, None));
        assert!(ws.report().is_none());
        assert_eq!(ws.scanned_at_ms(), None);
    }

    #[test]
    fn scan_requested_sets_scanning_flag() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::ScanRequested);
        assert!(ws.scanning());
    }

    #[test]
    fn scanned_ok_clears_scanning_and_updates_report() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::ScanRequested);
        update(&mut ws, Message::Scanned(1, Ok(sample_report())));
        assert!(!ws.scanning());
        assert!(ws.report().is_some());
        assert!(ws.scanned_at_ms().is_some());
    }

    #[test]
    fn scanned_err_clears_scanning_but_keeps_previous_report() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Loaded(1, Some(sample_report()), Some(1)));
        update(&mut ws, Message::ScanRequested);
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        assert!(!ws.scanning());
        assert!(ws.report().is_some(), "失败时应保留上一次成功结果");
        assert_eq!(ws.scanned_at_ms(), Some(1), "失败不应更新时间戳");
        assert_eq!(ws.scan_error(), Some("扫描失败"));
    }

    #[test]
    fn scan_requested_clears_previous_error() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        assert_eq!(ws.scan_error(), Some("扫描失败"));
        update(&mut ws, Message::ScanRequested);
        assert_eq!(ws.scan_error(), None);
    }

    #[test]
    fn scanned_ok_clears_previous_error() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        update(&mut ws, Message::Scanned(1, Ok(sample_report())));
        assert_eq!(ws.scan_error(), None);
    }
}
```

`dozer_codehealth::ProjectReport` 目前没有 `#[derive(Clone)]`（Task 3 只 derive 了 `Debug, PartialEq`）——`Message::Scanned`/`Message::Loaded` 按值携带它，`update` 函数体内部又要在测试里构造它两次，需要 `Clone`。回到 `crates/dozer-codehealth/src/report.rs`，把 `ProjectReport`（以及它内部持有的 `FunctionMetric`）的 derive 都加上 `Clone`：`#[derive(Debug, Clone, PartialEq)]`。（`FunctionMetric`/`FileMetric` 同步加 `Clone`，`HealthTier`/`Severity` 已经是 `Copy` 不需要改。）

- [ ] **Step 2: 运行测试确认失败（先跑一遍看清哪些是"缺 Clone"的编译错误，跟"逻辑错误"的失败分开看）**

```bash
cargo test -p dozer-app codehealth
```

Expected: 第一次很可能是编译错误（`ProjectReport` 未 `Clone`），按上面 Step 1 末尾的说明去 `dozer-codehealth` 补 `Clone`后再跑一次；补完后应该直接 PASS（`update` 是同批写的纯函数，不需要额外的"先失败"циkle）。

- [ ] **Step 3: 写 `aggregate.rs`（异步加载/扫描，模板抄 `extensions/usage/mod.rs::spawn_refresh`）**

```rust
//! 异步扫描/加载：仿 `extensions::usage::spawn_refresh` 的写法
//! （`crates/dozer-app/src/extensions/usage/mod.rs:119`）——`handle.spawn`
//! 起一个 tokio 任务，CPU 密集的本地扫描丢 `spawn_blocking`，dozerd
//! 往返走 `client`，结果经 `emit` 回灌成 `Message`。

use super::Message;
use dozer_codehealth::ProjectReport;
use std::path::PathBuf;

/// 面板打开时调用：只读落盘缓存，不触发扫描（spec：手动触发，不自动扫）。
pub fn spawn_load_cached(
    project_id: i64,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let cached = client.get_code_health_report(project_id).await.ok().flatten();
        let (report, scanned_at_ms) = match cached {
            Some(info) => {
                let report: Option<ProjectReport> =
                    serde_json::from_str(&info.report_json).ok();
                (report, Some(info.scanned_at_ms))
            }
            None => (None, None),
        };
        emit(Message::Loaded(project_id, report, scanned_at_ms));
    });
}

/// 点"扫描"按钮：本地跑 `dozer_codehealth::scan_project`（CPU/IO 密集,
/// `spawn_blocking`），成功后异步落盘到 dozerd，再回灌 UI。
pub fn spawn_scan(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = tokio::task::spawn_blocking(move || dozer_codehealth::scan_project(&project_path))
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("扫描任务被取消: {e}")));
        match result {
            Ok(report) => {
                if let Ok(report_json) = serde_json::to_string(&report) {
                    let _ = client
                        .save_code_health_report(
                            project_id,
                            report_json,
                            report.total_loc as u64,
                            report.total_functions as u64,
                            report.critical_functions as u64,
                            format!("{:?}", report.overall_tier),
                        )
                        .await;
                }
                emit(Message::Scanned(project_id, Ok(report)));
            }
            Err(e) => emit(Message::Scanned(project_id, Err(e.to_string()))),
        }
    });
}
```

`dozer_codehealth::ProjectReport`/`FunctionMetric`/`FileMetric`/`HealthTier`/`Severity` 目前没有 derive `Serialize`/`Deserialize`（Task 2/3 只 derive 了 `Debug, Clone, PartialEq`(, `Eq, PartialOrd, Ord`))。回到 `crates/dozer-codehealth`，给这 5 个类型都加上 `serde::{Serialize, Deserialize}` 的 derive，并给 crate 加 `serde` 依赖（`cd crates/dozer-codehealth && cargo add serde --features derive`）——落盘存的是整个 `ProjectReport` 的 JSON（见 Task 4 `CodeHealthReportInfo.report_json` 的字段文档），必须能序列化。

- [ ] **Step 4: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 这一步只保证 `extensions::codehealth` 模块自身编译通过——`mod.rs`/`aggregate.rs` 还没被 `extensions/mod.rs` 挂载，编译器不会主动检查它们，先手动加一行 `mod codehealth;`(`pub(crate) mod codehealth;`，对照 `extensions/mod.rs` 里 `usage` 是怎么声明的照抄可见性) 到 `crates/dozer-app/src/extensions/mod.rs` 让它参与编译，此声明在 Task 9 还会保留，不是本步骤临时加临时删。

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo test -p dozer-app codehealth
```

Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app crates/dozer-codehealth
git commit -m "feat(app): 代码健康度面板数据层（扫描/加载/落盘）"
```

---

### Task 8: 视图渲染——健康卡片 + 问题列表

**Files:**
- Create: `crates/dozer-app/src/extensions/codehealth/view.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`

**Interfaces:**
- Consumes: `WorkspaceState`（Task 7），`byteui::theme::color::{current, ColorTokens}`。
- Produces（供 Task 9 使用）：`pub fn content_pane<'a>(ws_state: &'a WorkspaceState, width: iced_widget::core::Length) -> iced_widget::core::Element<'a, Message>`

- [ ] **Step 1: 读一遍 `extensions/usage/view.rs` 的 `content_pane` 签名与整体结构，作为本文件的直接模板（字段级抄，仅替换成健康卡片 + 问题列表两块内容，不抄它的双栏 split/agent 筛选逻辑——spec 明确本面板只有一块可滚动内容，没有独立的 list pane）。**

这一步是纯阅读，不写代码。

- [ ] **Step 2: 写 `view.rs`**

```rust
//! 代码健康度面板渲染：健康卡片（等级徽章 + 摘要 + 扫描按钮）+ 问题列表
//! （按文件分组、按严重度排序的函数级排行榜）。单块可滚动内容，不像
//! `usage`/`conversations`/`agent` 那样有独立的 list pane + 分栏
//! （spec「UI 设计」：两块 UI 堆叠展示，不做三个独立视图）。

use super::{Message, WorkspaceState};
use byteui::theme::color::ColorTokens;
use dozer_codehealth::{FunctionMetric, HealthTier, ProjectReport, Severity};
use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row, scrollable, text, Column};
use std::collections::BTreeMap;

fn tier_color(tier: HealthTier, tokens: &ColorTokens) -> iced_widget::core::Color {
    match tier {
        HealthTier::Healthy => tokens.green,
        HealthTier::Watch => tokens.cyan,
        HealthTier::Critical => tokens.red,
    }
}

fn tier_label(tier: HealthTier) -> &'static str {
    match tier {
        HealthTier::Healthy => "健康",
        HealthTier::Watch => "关注",
        HealthTier::Critical => "警戒",
    }
}

fn health_card<'a>(report: &ProjectReport, scanned_at_ms: Option<u64>) -> Element<'a, Message> {
    let tokens = byteui::theme::color::current();
    let badge_color = tier_color(report.overall_tier, &tokens);
    let summary = format!(
        "核心代码 {} 行，{} 个函数存在明显结构问题",
        report.total_loc, report.critical_functions
    );
    let scanned_at = match scanned_at_ms {
        Some(ms) => format!("上次扫描：{}", format_ms(ms)),
        None => "尚未扫描".to_string(),
    };
    column![
        row![
            text(tier_label(report.overall_tier))
                .size(20)
                .color(badge_color),
            text(summary).size(14).color(tokens.body),
        ]
        .spacing(12),
        row![
            text(scanned_at).size(12).color(tokens.dim),
            button(text("扫描").size(13)).on_press(Message::ScanRequested),
        ]
        .spacing(12),
    ]
    .spacing(8)
    .padding(16)
    .into()
}

fn format_ms(ms: u64) -> String {
    // 面板本身不需要时区感知的复杂格式化——这里只做一个粗略的可读展示，
    // 精确格式化（本地时区/相对时间等）不是本设计的核心诉求，YAGNI。
    let secs = ms / 1000;
    format!("{secs}s epoch")
}

fn problem_row(f: &FunctionMetric) -> Element<'_, Message> {
    let tokens = byteui::theme::color::current();
    let color = match f.severity {
        Severity::Critical => tokens.red,
        Severity::Watch => tokens.cyan,
        Severity::Normal => tokens.dim,
    };
    row![
        text(&f.name).size(13).color(tokens.cream),
        text(format!("complexity={} loc={}", f.complexity_signal, f.loc))
            .size(12)
            .color(color),
    ]
    .spacing(8)
    .padding([2, 8])
    .into()
}

fn problem_list(report: &ProjectReport) -> Element<'_, Message> {
    let tokens = byteui::theme::color::current();
    let mut by_file: BTreeMap<&std::path::Path, Vec<&FunctionMetric>> = BTreeMap::new();
    for f in &report.functions {
        by_file.entry(f.file.as_path()).or_default().push(f);
    }
    let mut col = Column::new().spacing(4);
    for (path, functions) in by_file {
        col = col.push(
            text(path.display().to_string())
                .size(12)
                .color(tokens.dim),
        );
        for f in functions {
            col = col.push(problem_row(f));
        }
    }
    scrollable(col.padding(16)).into()
}

fn error_banner<'a>(message: &str, tokens: &ColorTokens) -> Element<'a, Message> {
    text(format!("扫描失败，请重试：{message}"))
        .size(12)
        .color(tokens.red)
        .into()
}

pub fn content_pane(ws_state: &WorkspaceState, width: Length) -> Element<'_, Message> {
    let tokens = byteui::theme::color::current();
    let error = ws_state.scan_error().map(|e| error_banner(e, &tokens));

    if ws_state.scanning() {
        // 扫描中：若已有上一次结果，仍展示它（不清空），只在顶部叠一条
        // "扫描中…"提示——比整块换成 loading 占位更不容易让用户以为数据
        // 丢了；若从没扫描过（`report()` 为 `None`），只显示 loading 提示。
        let scanning_text = text("扫描中…").size(14).color(tokens.body);
        return match ws_state.report() {
            Some(report) => container(column![
                scanning_text,
                health_card(report, ws_state.scanned_at_ms()),
                problem_list(report),
            ])
            .width(width)
            .into(),
            None => container(column![scanning_text]).width(width).into(),
        };
    }

    let Some(report) = ws_state.report() else {
        let mut col = column![
            text("这个项目还没有可分析的 Rust 代码，或者还没有扫描过。")
                .size(14)
                .color(tokens.body),
            button(text("扫描")).on_press(Message::ScanRequested),
        ]
        .spacing(12);
        if let Some(err) = error {
            col = col.push(err);
        }
        return container(col.padding(16)).width(width).into();
    };

    let mut col = column![health_card(report, ws_state.scanned_at_ms())];
    if let Some(err) = error {
        col = col.push(err);
    }
    col = col.push(problem_list(report));
    container(col).width(width).into()
}
```

具体 `iced_widget` 的 import 路径/`button`/`text`/`row`/`column`/`scrollable` 的确切调用方式（是否需要 `.style(..)`、`Length` 的确切 import 位置等）跟 `extensions/usage/view.rs` 顶部的 `use` 列表核对，若某个 widget 的调用签名跟这里写的不一致（iced 0.14 的 widget 函数签名有版本细节），以 `usage/view.rs` 里已经编译通过的实际写法为准，逐个替换成一致的调用方式而不是死抠本步骤给的示例代码。这一条适用于本步骤全部内容。

- [ ] **Step 3: 挂到 `mod.rs`**

```rust
mod view;
pub(crate) use view::content_pane;
```

- [ ] **Step 4: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译成功（不要求此刻已接入 rail/PanelKind——Task 7 Step 4 已经把 `codehealth` 模块挂进了 `extensions/mod.rs`，此处只是新增了它内部的 `view` 子模块）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(app): 代码健康度面板视图——健康卡片 + 问题列表"
```

---

### Task 9: 接入 `PanelKind::CodeHealth`（rail 图标、状态挂载、消息路由）

**Files:**
- Create: `crates/byteui/assets/icons/square-activity.svg`
- Modify: `crates/byteui/src/interaction/icons.rs`
- Modify: `crates/dozer-app/src/app/state.rs`
- Modify: `crates/dozer-app/src/chrome/rail.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `crates/dozer-app/src/app/view.rs`
- 以及编译器报出的其它 `PanelKind` 穷尽匹配点（见 Step 7）

**Interfaces:**
- Consumes: `codehealth::{WorkspaceState, Message, update, content_pane}`（Task 7/8）。
- Produces: 可从 GUI 右侧图标栏点开的"代码健康度"面板。

- [ ] **Step 1: 加 Lucide 图标资产**

`crates/byteui/assets/icons/square-activity.svg`：

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <rect width="18" height="18" x="3" y="3" rx="2" />
  <path d="M17 12h-2l-2 5-2-10-2 5H7" />
</svg>
```

（格式对齐仓库里已有的 `bar-chart-3.svg`：无 `class` 属性、无许可证注释行。）

- [ ] **Step 2: 注册 `IconKind::SquareActivity`**

`crates/byteui/src/interaction/icons.rs`：`IconKind` 枚举（约第 126 行 `BarChart3,` 旁）加一行 `SquareActivity,`；渲染匹配（约第 284 行 `IconKind::BarChart3 => include_bytes!(..)` 旁）加：

```rust
            IconKind::SquareActivity => {
                include_bytes!("../../assets/icons/square-activity.svg")
            }
```

- [ ] **Step 3: 编译确认（这一步应该报"非穷尽匹配"错误——这是预期的，Rust 编译器会精确列出所有需要补 `PanelKind::CodeHealth` 分支的位置，比手工 grep 更可靠）**

```bash
cargo build -p byteui
```

Expected: 编译成功（`IconKind` 的匹配已经补全，这一步单独验证图标注册没打错字）。

- [ ] **Step 4: 加 `PanelKind::CodeHealth` 枚举值**

`crates/dozer-app/src/app/state.rs`，`enum PanelKind`（第 21-32 行）加一行：

```rust
pub enum PanelKind {
    Files,
    GitLog,
    Todo,
    Project,
    Database,
    Ssh,
    Web,
    Agent,
    Conversations,
    Usage,
    CodeHealth,
}
```

`default_side()`（第 38-49 行）里 `Self::Agent | Self::Conversations | Self::Usage => Side::Right,` 改成：

```rust
            Self::Agent | Self::Conversations | Self::Usage | Self::CodeHealth => Side::Right,
```

- [ ] **Step 5: 编译，用编译器报错定位所有需要补 `PanelKind::CodeHealth` 分支的穷尽匹配**

```bash
cargo build -p dozer-app 2>&1 | grep -A2 "non-exhaustive"
```

按报错逐个补，以下是已知会命中的位置及本任务推荐的填法（**理由**：`CodeHealth` 是右侧栏、单块内容无独立 list pane、不含任何 wry webview——除了 rail 图标/tooltip 和 view.rs 渲染入口这两处需要真正的新逻辑，其余穷尽匹配点全部照抄同类右侧面板 `Usage`/`Agent` 里"无 webview 遮挡"的值即可，不需要照抄它们"有 list pane split"的那部分，因为 `CodeHealth` 根本没有 list pane）：

  - `crates/dozer-app/src/chrome/rail.rs`：
    - `RailLayout::default()`（约第 113-128 行）的 `right: vec![..]` 追加 `PanelKind::CodeHealth`。
    - `sanitize_rail_layout`（约第 134-145 行）里两处 `!= 10` 改成 `!= 11`（面板总数从 10 变 11）；同函数上方文档注释（第 130-133 行）"10 个不重复"改成"11 个不重复"。
    - `panel_meta`（约第 485-498 行）加一行：`PanelKind::CodeHealth => (icons::IconKind::SquareActivity, "代码健康度"),`。
    - 文件头注释（第 2 行）"10 个面板"改"11 个面板"；`panel_meta` 上方文档注释（第 482-484 行）同改。
    - `RailLayout` 相关的字段级文档（约第 87 行）"10 个面板"同改。
    - 约第 670/680 行的测试：`assert_eq!(all.len(), 10, ..)` 改成 `11`，文档注释同改。
  - `crates/dozer-app/src/webview_geometry.rs`：所有 `PanelKind::Usage`（约第 127/259/855/882/963 行）出现的穷尽匹配分支，追加 `PanelKind::CodeHealth` 到同一个分支（都是 `(0.0, 0.0, 0.0, 0.0)` 零遮挡——`CodeHealth` 面板纯 iced 渲染，不含 wry webview，跟 `Usage`/`Agent` 特征一致）。
  - `crates/dozer-app/src/app/layout.rs`：约第 545/588/959 行的 `PanelKind::Usage` 分支——这几处是"分栏宽度"相关（`usage_split`/`PanelDims`），`CodeHealth` 没有分栏，不需要新增一个 `codehealth_split` 字段；按编译器报错的分支类型，若是穷尽匹配就加 `PanelKind::CodeHealth =>` 返回一个安全默认值（`None`/该分支原有返回类型的"不参与分栏"语义值，具体参照该 match 里其它"没有 list pane"的面板——如 `Project`/`Files` 等单块面板——是怎么处理的，照抄那个值而不是 `Usage` 的值，因为 `Usage` 恰好是"有分栏"的那类）。
  - `crates/dozer-app/src/app/app.rs`：约第 5064/5081 行的 `PanelKind::Usage`——核对上下文，若是"计算 mirrored 状态"这类通用逻辑，直接追加到同一分支；若不是穷尽匹配（只是具体调用某个函数时传的字面量）则不需要改。
  - `crates/dozer-app/src/app/update.rs`：
    - 约第 3296/3313 行 `PanelKind::Usage => self.dims.usage_list_collapsed` 这类"list collapse 状态"匹配——`CodeHealth` 没有独立 list pane，加 `PanelKind::CodeHealth => false,`（永远不算"收起"，因为它只有一块内容，没有可收起的 list 部分）；`PanelKind::Usage => &mut self.dims.usage_list_collapsed` 这类返回可变引用的匹配没法直接返回字面量 `false`——若编译器在这类位置报错，说明这个字段设计假设"每个右侧面板都有一个可变的 collapse 存储位"，本任务不新增 `codehealth_list_collapsed: bool` 字段，而是让 `PanelKind::CodeHealth` 分支复用panel 自身不需要被"收起"这个事实：具体做法留给实现者按该函数返回类型决定（如果必须返回 `&mut bool`，就在 `AppDims`/相应结构体上加一个 `codehealth_list_collapsed: bool` 字段，即使它永远是 `false` 也不影响功能，只是多一个从不被翻转的字段——这是比"伪造一个不属于任何真实状态的可变引用"更安全的选择）。
    - 约第 184 行 `self.toggle_panel_list_collapse(PanelKind::Usage);`——这是具体调用点不是穷尽匹配，不需要改。
    - 约第 3236 行 `PanelKind::Usage => self.with_focused_project(|ws, io| { .. })`——这是"切到面板时做什么"的分发（`PanelSelect` 处理逻辑的一部分）。`CodeHealth` 在这里加一个新分支，调用 Task 7 的 `codehealth::spawn_load_cached`（**不是** `spawn_scan`——面板打开只读缓存，不自动扫描，这是 spec 明确要求的行为，也是本面板跟 `Usage` 面板"打开即自动扫"最大的行为差异）：
      ```rust
      PanelKind::CodeHealth => self.with_focused_project(|ws, io| {
          let Some(project_id) = ws.project_id else { return; };
          let proxy = io.proxy.clone();
          crate::extensions::codehealth::spawn_load_cached(
              project_id,
              &io.client,
              &io.handle,
              move |m| {
                  let _ = proxy.send_event(Message::CodeHealth(m));
              },
          );
      }),
      ```
      （`ws.project_id`/`io.proxy`/`io.client`/`io.handle` 的具体字段名以 `PanelKind::Usage` 分支实际用到的字段名为准核对——这几行是照着"这个分支在做什么"复述出来的，不是直接抄某一行代码，实现者要打开 `update.rs:3236` 附近实际的 `PanelKind::Usage` 分支核对字段名拼写。）
  - `crates/dozer-app/src/app/view.rs`：约第 1169 行 `PanelKind::Usage => { .. }` 旁边（不是仿它的双栏 split 逻辑，是仿更简单的单块面板，若同一个 match 里有 `PanelKind::Project` 这类单块面板可参考）加：
    ```rust
    PanelKind::CodeHealth => container(
        codehealth::content_pane(&ws.codehealth, Length::Fill),
    )
    .into(),
    ```
    `zone_pane_border`/`container` 的具体包法照当前 match 分支里其它"单块无分栏"面板的写法，不照抄 `Usage` 的双栏写法。

- [ ] **Step 6: `Workspace` 挂 `codehealth` 字段**

`crates/dozer-app/src/workspace/state.rs`，紧邻 `pub(crate) usage: usage::WorkspaceState,`（约第 358-359 行）加：

```rust
    /// 代码健康度面板 per-project 状态——见 `extensions::codehealth::WorkspaceState`。
    pub(crate) codehealth: codehealth::WorkspaceState,
```

顶部 `use crate::extensions::usage;` 旁加 `use crate::extensions::codehealth;`；`Workspace` 构造处（约第 605 行 `usage: usage::WorkspaceState::default(),`）旁加 `codehealth: codehealth::WorkspaceState::default(),`；项目切换重置处（约第 983 行 `self.usage = usage::WorkspaceState::default();`）旁加对应重置行。

- [ ] **Step 7: 顶层 `Message` 包装**

`crates/dozer-app/src/app/message.rs`，`enum Message` 里找 `Usage(usage::Message)`（同样字面存在——仿 `Request::AddBookmark` 时用过的"找同类既有 variant 照抄"手法，这里是找一个已有的 `Xxx(xxx::Message)` 包装 variant，未必刚好叫 `Usage`，以实际存在的那个为准）旁加：

```rust
    CodeHealth(crate::extensions::codehealth::Message),
```

`app/update.rs` 顶层 `Message::Usage(m) => { .. }` 分发处（转发给 `ws.codehealth`/调用 `codehealth::update`）旁加对应分支：

```rust
        Message::CodeHealth(m) => self.with_focused_project(|ws, _io| {
            crate::extensions::codehealth::update(&mut ws.codehealth, m);
        }),
```

（`with_focused_project` 的确切用法照抄 `Message::Usage` 分支实际写法，字段名/闭包签名以那里为准。）

- [ ] **Step 8: 编译确认——反复跑，把每一条 "non-exhaustive" 错误消掉**

```bash
cargo build -p dozer-app 2>&1 | grep -B2 "non-exhaustive\|error\[" | head -100
```

Expected: 最终编译成功，0 error。这一步预计要跑很多轮（每轮补一批分支再重新编译），是这个任务里最耗时的部分,属于计划内。

- [ ] **Step 9: 跑全部测试**

```bash
cargo test -p dozer-app
cargo test -p byteui
```

Expected: PASS。特别关注 Task 9 Step 5 提到的 `rail.rs` 里"10 个面板"相关的既有测试（原本断言 `== 10` 的那个），必须已经改成 `11` 并通过。

- [ ] **Step 10: `cargo clippy` + `cargo fmt` 检查**

```bash
cargo clippy --all-targets 2>&1 | tail -50
cargo fmt --check
```

Expected: 无新增 clippy 警告；`fmt --check` 无差异（若有差异跑 `cargo fmt` 直接改）。

- [ ] **Step 11: Commit**

```bash
git add crates/byteui crates/dozer-app
git commit -m "feat(app): PanelKind::CodeHealth 接入右侧图标栏"
```

---

### Task 10: 问题列表点击跳转到 Files 面板定位

**Files:**
- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/preview/view.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`

**Interfaces:**
- Consumes: `CodeView::move_cursor_to((usize, usize))`（既有，`code_editor/mod.rs:330`）、`App::preview_open_path`（既有，`app/update.rs:3386`）、`PreviewTab`（既有，`preview/state.rs:8-48`）。
- Produces: 点击 `codehealth::view::problem_row` 一行 → 切到 Files 面板、打开对应文件、光标落在函数起始行。

- [ ] **Step 1: `PreviewTab` 加 `pending_jump_line` 字段**

`crates/dozer-app/src/preview/state.rs`，`PreviewTab` 结构体（第 8-48 行）加一个字段：

```rust
    /// 由外部面板（目前只有代码健康度面板）请求的"打开后立即跳转到这一
    /// 行"——`Some` 只在"这个 tab 刚被新建、还在 `loading` 中"的窗口期内
    /// 有意义，`apply_native_load` 收到结果、把 `editor` 填上的那一刻立刻
    /// `take()` 消费掉。已经打开且 `editor` 已就绪的 tab 不走这个字段，
    /// 直接同步调用 `CodeView::move_cursor_to`。
    pub pending_jump_line: Option<usize>,
```

同 `Debug` 手写实现（第 50-66 行）加一行 `.field("pending_jump_line", &self.pending_jump_line)`。

- [ ] **Step 2: 编译，用编译器报错找到全部 `PreviewTab { .. }` 字面量构造点，逐个补 `pending_jump_line: None`**

```bash
cargo build -p dozer-app 2>&1 | grep -B2 "missing field"
```

按报错逐个加 `pending_jump_line: None,`（`insert_loading_tab`、`Blank` 占位 tab 构造等所有既有构造点）。

- [ ] **Step 3: `apply_native_load` 消费 `pending_jump_line`**

`crates/dozer-app/src/preview/view.rs`，`apply_native_load`（第 292 行起）里 `tab.editor = Some(load.view)` 那一行之后加：

```rust
                if let Some(line) = tab.pending_jump_line.take() {
                    if let Some(editor) = tab.editor.as_mut() {
                        editor.move_cursor_to((line, 0));
                    }
                }
```

（确认变量名 `tab`/`load` 是否跟这里假设的一致——以 `apply_native_load` 实际的局部变量名为准调整，不是死抄。）

- [ ] **Step 4: `App` 加 `preview_open_path_at` / 改 `preview_open_path` 委托**

`crates/dozer-app/src/app/update.rs`，把 `preview_open_path`（第 3386 行起）的函数体搬进一个新的 `preview_open_path_at`，原 `preview_open_path` 变成薄委托：

```rust
    pub(crate) fn preview_open_path(&mut self, path: PathBuf) {
        self.preview_open_path_at(path, None);
    }

    pub(crate) fn preview_open_path_at(&mut self, path: PathBuf, target_line: Option<usize>) {
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        let Some(project_id) = self.active_project_id else {
            return;
        };
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());

            if let Some(idx) = ws.preview.find_existing_file_tab(&path) {
                ws.preview.select(idx);
                if let Some(line) = target_line {
                    if let Some(tab) = ws.preview.tabs_mut().get_mut(idx) {
                        if let Some(editor) = tab.editor.as_mut() {
                            editor.move_cursor_to((line, 0));
                        } else {
                            tab.pending_jump_line = Some(line);
                        }
                    }
                }
            } else if crate::preview::is_native_editor_candidate(&path) {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                let tab_id = ws
                    .preview
                    .insert_loading_tab(crate::preview::TabKind::File(path.clone()), title);
                if let Some(line) = target_line {
                    if let Some(tab) = ws.preview.tabs_mut().iter_mut().find(|t| t.id == tab_id) {
                        tab.pending_jump_line = Some(line);
                    }
                }
                let proxy = io.proxy.clone();
                let load_path = path.clone();
                io.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        crate::preview::read_and_build_native_editor(&load_path)
                            .map(crate::preview::NativeEditorLoadHandle::new)
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ =
                        proxy.send_event(Message::PreviewFileLoaded(project_id, tab_id, result));
                });
            } else {
                ws.preview.open_path(path.clone());
                // 非原生编辑器候选(webview/表格类)无法跳转光标,target_line
                // 静默忽略——这类文件本来就不会是代码健康度面板的分析对象
                // (只扫 .rs),实践中不会走到这条分支。
            }
        });
    }
```

（`avail_w` 变量在原函数里若被用到别处（比如设置 tab 栏宽度快照），要看原函数余下部分是怎么用它的——这里省略了原函数除 `if let Some(idx) = ..` 之后不涉及跳转逻辑的部分，实现者要把原 `preview_open_path` 函数体**除了** `find_existing_file_tab`/`insert_loading_tab` 这两个分支内部新加的跳转逻辑之外的其它部分原样保留，不要漏掉。）

- [ ] **Step 5: `App` 加一个"切到 Files + 打开定位"的组合入口**

`app/update.rs` 里紧邻 `preview_open_path_at` 加：

```rust
    /// 代码健康度面板"点击函数跳转"入口：确保 Files 面板可见，再打开该
    /// 文件并跳到目标行。`panel_select` 在已选中同一面板时会触发"收起/
    /// 展开"的 toggle 副作用（见 `panel_select` 文档），这里先判断避免
    /// 误触。
    pub(crate) fn code_health_open_location(&mut self, path: PathBuf, line: usize) {
        if self.right_view != PanelKind::Files {
            self.panel_select(PanelKind::Files);
        }
        self.preview_open_path_at(path, Some(line));
    }
```

- [ ] **Step 6: 新增消息与分发**

`app/message.rs` 加：

```rust
    /// 代码健康度面板"点击函数"：打开该文件并跳到 `usize`(目标行,
    /// 1-based——跟 `FunctionMetric.start_line` 同口径)。
    CodeHealthOpenLocation(PathBuf, usize),
```

`app/update.rs` 顶层 `match message` 里加：

```rust
            Message::CodeHealthOpenLocation(path, line) => {
                self.code_health_open_location(path, line);
            }
```

- [ ] **Step 7: `codehealth::view` 问题行接上点击**

`crates/dozer-app/src/extensions/codehealth/view.rs` 的 `problem_row` 改成接受 `MouseArea`（或该项目里 clickable 行的既有惯例 widget——核对 `usage/view.rs` 明细行是不是纯展示、还是 `extensions/files/tree.rs` 这类"树行点击"的写法更贴近，选用后者，因为本行明确需要点击交互）：

```rust
fn problem_row(f: &FunctionMetric) -> Element<'_, Message> {
    let tokens = byteui::theme::color::current();
    let color = match f.severity {
        Severity::Critical => tokens.red,
        Severity::Watch => tokens.cyan,
        Severity::Normal => tokens.dim,
    };
    let content = row![
        text(&f.name).size(13).color(tokens.cream),
        text(format!("complexity={} loc={}", f.complexity_signal, f.loc))
            .size(12)
            .color(color),
    ]
    .spacing(8)
    .padding([2, 8]);
    iced_widget::mouse_area(content)
        .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
        .into()
}
```

`Message`（`codehealth::Message`，Task 7 定义）加一个新 variant：

```rust
    /// 问题列表点击某函数：文件路径 + 目标行(1-based)。由内核（`app/update.rs`
    /// 的 `Message::CodeHealth` 分发处）拦截转成顶层 `Message::CodeHealthOpenLocation`，
    /// 不进入本模块自己的 `update`（同 `usage::Message::ToggleListCollapse`
    /// "由内核拦截处理"的既有模式——见 `usage/mod.rs:105-107`）。
    OpenLocation(std::path::PathBuf, usize),
```

`codehealth::update`（Task 7 `mod.rs`）里对应加：

```rust
        Message::OpenLocation(..) => {
            unreachable!("由内核拦截处理,见 codehealth::Message::OpenLocation 文档")
        }
```

`app/update.rs` 的 `Message::CodeHealth(m) => ..` 分发处（Task 9 Step 7 加的）要在转发给 `codehealth::update` 之前先拦截这个 variant：

```rust
        Message::CodeHealth(crate::extensions::codehealth::Message::OpenLocation(path, line)) => {
            self.code_health_open_location(path, line);
        }
        Message::CodeHealth(m) => self.with_focused_project(|ws, _io| {
            crate::extensions::codehealth::update(&mut ws.codehealth, m);
        }),
```

（这条会替换掉 Task 9 Step 7 里加的那个单一 `Message::CodeHealth(m) => ..` 分支，改成上面两条——注意 match 分支顺序，更具体的模式要写在前面。）

- [ ] **Step 8: 编译 + 测试**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
```

Expected: 编译成功，测试 PASS。

- [ ] **Step 9: `cargo clippy` + `cargo fmt`**

```bash
cargo clippy --all-targets 2>&1 | tail -50
cargo fmt --check
```

Expected: 无新增警告；无格式差异。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(app): 代码健康度面板问题列表点击跳转到 Files 面板定位"
```

---

### Task 11: 人工 GUI 验收

**Files:** 无代码改动。

- [ ] **Step 1: 启动 GUI**

```bash
cargo run -p dozer-app
```

- [ ] **Step 2: 验收清单**

在一个已打开的 dozer 项目里：

1. 右侧图标栏出现新的"代码健康度"图标（`square-activity` 造型），点击可打开面板。
2. 首次打开：面板显示"这个项目还没有可分析的 Rust 代码，或者还没有扫描过"+"扫描"按钮（该项目此前没有落盘记录的前提下）。
3. 点"扫描"：短暂 loading 后出现健康卡片（等级徽章 + 摘要文字 + 上次扫描时间）与下方按文件分组的函数问题列表。
4. 关闭面板、重新打开：不重新扫描，直接展示上次结果（落盘持久化生效）。
5. 点问题列表里任意一个函数：右侧切到 Files 面板，对应文件被打开，光标/视口定位到该函数起始行附近。
6. 切换到 dozer 仓库自身（dogfooding）：扫描后应该能在结果里看到 `crates/dozer-app/src/app/update.rs` 的 `update` 函数排在复杂度榜单前列（对照 spike 阶段已经验证过的真实发现）。
7. 健康卡片配色：确认徽章颜色是绿/青/红三色之一，不是金色。
8. 面板内文字确认不是等宽代码字体（应为系统默认字体——只有 code editor/终端用等宽字体）。

- [ ] **Step 3: 记录验收结果**

若发现问题，记录现象（不在本计划里修复——回到 writing-plans 或直接现场判断是否需要追加任务）。若全部通过，本计划完成。

---
