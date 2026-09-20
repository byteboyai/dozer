# UI 复杂度检测 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给代码健康度体系新增第二条腿——UI 一致性检测：颜色/边距/字体硬编码、组件树嵌套深度、事件回调密度、组件化重复结构，六个维度全部扩展 `dozer-codehealth` 现有的 `ProjectReport`/`FunctionMetric`，面板问题列表下方新增"UI 一致性"分类分区展示。

**Architecture:** 复用 `dozer-codehealth` 已有的 ast-grep + 文件遍历流水线，新增 `ui_metrics.rs` 模块承载六个维度的检测逻辑与严重度分档函数；`FunctionMetric` 加两个新字段（嵌套深度、事件回调数），`ProjectReport` 加十个新字段（三类字面量发现 + 各自严重度、两个调色板统计、重复簇列表及其严重度、嵌套深度/事件回调的"最严重违规者"严重度、汇总 `ui_tier`）。`dozer-app` 侧只读这些字段渲染，不重复计算阈值逻辑。

**Tech Stack:** Rust, `ast-grep-core` 0.45（`Pattern`/`Matcher` API，必须预编译复用，见 Global Constraints）, `iced_widget` 0.14。

**Spec:** `docs/superpowers/specs/2026-09-20-ui-complexity-detection-design.md`

## Global Constraints

- **在独立分支上开发，不直接提交 main**：`feature/ui-complexity-detection`，完成后提请审阅，审阅通过后再合并回 main。
- **性能约束（spike 已验证的硬性要求）**：所有 `ast_grep_core::matcher::Pattern` 必须在文件遍历循环**外**用 `Pattern::new(src, lang)` 预编译一次并复用；**禁止**把裸 `&str` pattern 字符串直接传给 `find_all`/`find`（`impl Matcher for str` 会在每次 `match_node` 调用时重新编译 pattern，在树的每个节点上都重新编译一次，单文件就能从毫秒级退化到 6+ 秒）。任何新增的 `find_all(...)` 调用点，参数必须是预编译好的 `&Pattern`，不能是格式化出来的字符串字面量。
- **颜色规则**：任何 `Color::from_rgb($$$)`/`Color::from_rgba($$$)`/`Color::from_rgb8($$$)` 构造调用本身就是一次发现，不需要判断参数是不是字面量。
- **边距/字体规则**：`.padding($X)`/`.spacing($X)`/`.font($X)`（`.font` 匹配 `$RECV.font($X)`，另外单独匹配 `Font::with_name($X)`）**要**区分 `$X` 是字面量还是变量/常量引用——只有字面量才算一次发现。边距的字面量 kind 集合是 `{integer_literal, float_literal, array_expression, unary_expression}`；字体的字面量 kind 是 `{string_literal}`。
- **嵌套深度用文本扫描，不用 AST**：tree-sitter-rust 不会把嵌套在另一个宏参数里的宏调用识别成独立的 `macro_invocation` 节点（spike 已验证：`column![row![...]]` 内层 `row!` 找不到），因此 `widget_nesting_depth` 必须实现成对函数源码文本做 `row![`/`column![`方括号深度扫描的小状态机，不能尝试用 AST 节点种类过滤。
- **组件化重复结构只看顶层**：结构指纹只对不嵌套在另一个宏参数里的顶层 `row!`/`column!` 调用计算，遇到目标宏就停止下钻（子结构不重复计入）。
- **严重度统一复用 `HealthTier`**（`dozer_codehealth::HealthTier`），不新建平行的严重度枚举。
- **不做按路径排除清单**、**不做每项目可配置阈值**——阈值全部是写死常量（见下表），标注"凭经验定，可调"。
- 面板新增区块的配色只用现有 `byteui::theme::color::ColorTokens` 字段，不新增 token，不用 `gold`；文字不显式设置 `.font(...)`，沿用默认（同既有 codehealth 面板惯例）。

**严重度阈值常量表**（全部写死，来源见 spec"指标定义"一节）：

| 维度 | Healthy | Watch | Critical |
|---|---|---|---|
| 颜色（项目级发现总数） | `0` | `1..=15` | `>15` |
| 边距（项目级发现总数） | `0..=10` | `11..=50` | `>50` |
| 字体（项目级发现总数） | `0` | `1..=5` | `>5` |
| 嵌套深度（单函数） | `<=2` | `3..=4` | `>4` |
| 事件回调密度（单函数） | `0..=3` | `4..=6` | `>6` |
| 重复簇（单簇出现次数，`<3` 不成簇不参与分档） | 无 | `3..=5` | `>5` |

---

### Task 1: `ui_metrics.rs` 骨架——数据结构 + 六个严重度分档函数

**Files:**
- Create: `crates/dozer-codehealth/src/ui_metrics.rs`
- Modify: `crates/dozer-codehealth/src/lib.rs`

**Interfaces:**
- Consumes: `crate::report::HealthTier`（已有，`pub` 且实现 `Ord`）。
- Produces（供 Task 2-7 使用）：
  - `pub struct RawLiteralFinding { pub file: PathBuf, pub line: usize, pub snippet: String }`（`#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`，同 `FunctionMetric` 的 derive 惯例）
  - `pub struct DuplicateCluster { pub occurrences: Vec<(PathBuf, usize)>, pub node_count: usize }`（同上 derive）
  - `pub fn color_tier(count: usize) -> HealthTier`
  - `pub fn spacing_tier(count: usize) -> HealthTier`
  - `pub fn font_tier(count: usize) -> HealthTier`
  - `pub fn nesting_depth_tier(depth: usize) -> HealthTier`
  - `pub fn event_handler_tier(count: usize) -> HealthTier`
  - `pub fn cluster_tier(occurrences: usize) -> HealthTier`

- [ ] **Step 1: 写失败测试**

`crates/dozer-codehealth/src/ui_metrics.rs`：

```rust
use crate::report::HealthTier;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawLiteralFinding {
    pub file: PathBuf,
    pub line: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateCluster {
    pub occurrences: Vec<(PathBuf, usize)>,
    pub node_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_tier_boundaries() {
        assert_eq!(color_tier(0), HealthTier::Healthy);
        assert_eq!(color_tier(1), HealthTier::Watch);
        assert_eq!(color_tier(15), HealthTier::Watch);
        assert_eq!(color_tier(16), HealthTier::Critical);
    }

    #[test]
    fn spacing_tier_boundaries() {
        assert_eq!(spacing_tier(10), HealthTier::Healthy);
        assert_eq!(spacing_tier(11), HealthTier::Watch);
        assert_eq!(spacing_tier(50), HealthTier::Watch);
        assert_eq!(spacing_tier(51), HealthTier::Critical);
    }

    #[test]
    fn font_tier_boundaries() {
        assert_eq!(font_tier(0), HealthTier::Healthy);
        assert_eq!(font_tier(1), HealthTier::Watch);
        assert_eq!(font_tier(5), HealthTier::Watch);
        assert_eq!(font_tier(6), HealthTier::Critical);
    }

    #[test]
    fn nesting_depth_tier_boundaries() {
        assert_eq!(nesting_depth_tier(2), HealthTier::Healthy);
        assert_eq!(nesting_depth_tier(3), HealthTier::Watch);
        assert_eq!(nesting_depth_tier(4), HealthTier::Watch);
        assert_eq!(nesting_depth_tier(5), HealthTier::Critical);
    }

    #[test]
    fn event_handler_tier_boundaries() {
        assert_eq!(event_handler_tier(3), HealthTier::Healthy);
        assert_eq!(event_handler_tier(4), HealthTier::Watch);
        assert_eq!(event_handler_tier(6), HealthTier::Watch);
        assert_eq!(event_handler_tier(7), HealthTier::Critical);
    }

    #[test]
    fn cluster_tier_boundaries() {
        assert_eq!(cluster_tier(3), HealthTier::Watch);
        assert_eq!(cluster_tier(5), HealthTier::Watch);
        assert_eq!(cluster_tier(6), HealthTier::Critical);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests
```

Expected: 编译失败（六个 `*_tier` 函数尚未定义，`mod ui_metrics` 也还没在 `lib.rs` 里声明）。

- [ ] **Step 3: 声明模块 + 实现六个分档函数**

在 `crates/dozer-codehealth/src/lib.rs` 加一行 `mod ui_metrics;`（放在 `mod report;` 后面）。

在 `ui_metrics.rs` 里追加：

```rust
/// spec「颜色硬编码」：`0` Healthy，`1..=15` Watch，`>15` Critical。
pub fn color_tier(count: usize) -> HealthTier {
    if count > 15 {
        HealthTier::Critical
    } else if count >= 1 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「边距硬编码」：`0..=10` Healthy，`11..=50` Watch，`>50` Critical。
pub fn spacing_tier(count: usize) -> HealthTier {
    if count > 50 {
        HealthTier::Critical
    } else if count >= 11 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「字体硬编码」：`0` Healthy，`1..=5` Watch，`>5` Critical。
pub fn font_tier(count: usize) -> HealthTier {
    if count > 5 {
        HealthTier::Critical
    } else if count >= 1 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「组件树嵌套深度」：`<=2` Healthy，`3..=4` Watch，`>4` Critical。
pub fn nesting_depth_tier(depth: usize) -> HealthTier {
    if depth > 4 {
        HealthTier::Critical
    } else if depth >= 3 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「事件回调密度」：`0..=3` Healthy，`4..=6` Watch，`>6` Critical。
pub fn event_handler_tier(count: usize) -> HealthTier {
    if count > 6 {
        HealthTier::Critical
    } else if count >= 4 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「组件化重复结构」：`3..=5` Watch，`>5` Critical。调用方保证只对
/// `occurrences >= 3`（已成簇）的情况调用，`<3` 不构成簇、不会进这个函数。
pub fn cluster_tier(occurrences: usize) -> HealthTier {
    if occurrences > 5 {
        HealthTier::Critical
    } else {
        HealthTier::Watch
    }
}
```

`HealthTier` 目前是 `pub enum` 但不是 `pub(crate)` 限定——`report.rs` 里已经 `pub use` 过，`ui_metrics.rs` 用 `crate::report::HealthTier` 引用即可，不需要改 `report.rs`。

把 `crates/dozer-codehealth/src/lib.rs` 改成：

```rust
mod function_metric;
mod report;
mod ui_metrics;

pub use function_metric::{FunctionMetric, Severity, functions_in_source, severity_for};
pub use report::{
    FileMetric, HealthTier, ProjectReport, density_tier, file_metric, scale_tier, scan_project,
};
pub use ui_metrics::{
    DuplicateCluster, RawLiteralFinding, color_tier, cluster_tier, event_handler_tier, font_tier,
    nesting_depth_tier, spacing_tier,
};
```

（`Patterns`/`find_color_findings` 等 Task 2-4 才新增的项不在这里导出——`dozer-app` 只消费 `ProjectReport` 已经算好的字段和这六个 `*_tier` 函数，不需要自己调用检测函数或持有 `Patterns`，见 Task 7。）

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests
```

Expected: PASS，6 个测试全过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-codehealth/src/ui_metrics.rs crates/dozer-codehealth/src/lib.rs
git commit -m "feat(codehealth): UI 复杂度六维度严重度分档函数 + 数据结构骨架"
```

---

### Task 2: 颜色/边距/字体字面量检测

**Files:**
- Modify: `crates/dozer-codehealth/src/ui_metrics.rs`

**Interfaces:**
- Consumes: `ast_grep_core::{Doc, Node, matcher::Pattern}`、`ast_grep_language::SupportLang`（新增依赖用法，`Cargo.toml` 已有这两个包，不需要改依赖）。
- Produces（供 Task 4/6 使用）：
  - `pub struct Patterns { color_ctors: [Pattern; 3], padding: Pattern, spacing: Pattern, font_method: Pattern, font_with_name: Pattern, event_handlers: [Pattern; 5] }`（`event_handlers` 字段本任务先占位声明，Task 3 才真正用到；`Patterns` 不实现 `Clone`/`Debug`——`Pattern` 本身不一定支持，用不到）
  - `pub fn Patterns::compile(lang: SupportLang) -> Patterns`
  - `pub fn find_color_findings(root: &Node<'_, impl Doc>, patterns: &Patterns, file: &Path) -> Vec<RawLiteralFinding>`
  - `pub fn find_spacing_findings(root: &Node<'_, impl Doc>, patterns: &Patterns, file: &Path) -> Vec<RawLiteralFinding>`
  - `pub fn find_font_findings(root: &Node<'_, impl Doc>, patterns: &Patterns, file: &Path) -> Vec<RawLiteralFinding>`

- [ ] **Step 1: 写失败测试**

追加到 `ui_metrics.rs`（`#[cfg(test)] mod tests` 内，`use super::*;` 已经在，需要额外 `use ast_grep_language::{LanguageExt, SupportLang};` 和 `use std::path::Path;`）：

```rust
    fn parse(src: &str) -> ast_grep_core::AstGrep<impl ast_grep_core::Doc> {
        SupportLang::Rust.ast_grep(src)
    }

    #[test]
    fn find_color_findings_matches_constructor_calls() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse("fn f() { let c = Color::from_rgb(0.1, 0.2, 0.3); }");
        let findings = find_color_findings(&root.root(), &patterns, Path::new("a.rs"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn find_color_findings_ignores_field_access() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse("fn f() { let c = tokens.red; }");
        let findings = find_color_findings(&root.root(), &patterns, Path::new("a.rs"));
        assert!(findings.is_empty());
    }

    #[test]
    fn find_spacing_findings_flags_literal_not_variable() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse("fn f() { col.padding(16).spacing(gap); }");
        let findings = find_spacing_findings(&root.root(), &patterns, Path::new("a.rs"));
        // 只有 .padding(16) 的 16 是字面量,.spacing(gap) 的 gap 是变量引用,不算。
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].snippet, "16");
    }

    #[test]
    fn find_font_findings_flags_string_literal_not_const_ref() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse(
            "fn f() { let a = Font::with_name(\"JetBrains Mono\"); let b = Font::with_name(CODE_FONT_FAMILY); }",
        );
        let findings = find_font_findings(&root.root(), &patterns, Path::new("a.rs"));
        // 裸字符串字面量算一次发现,引用常量(CODE_FONT_FAMILY,identifier)不算。
        assert_eq!(findings.len(), 1);
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests -- find_color find_spacing find_font
```

Expected: 编译失败（`Patterns`/`find_color_findings`/`find_spacing_findings`/`find_font_findings` 尚未定义）。

- [ ] **Step 3: 实现 `Patterns` 与三个检测函数**

在 `ui_metrics.rs` 追加（`use` 段补上 `ast_grep_core::matcher::Pattern`、`ast_grep_language::SupportLang`）：

```rust
use ast_grep_core::matcher::Pattern;
use ast_grep_language::SupportLang;
use std::path::Path;

const SPACING_LITERAL_KINDS: &[&str] = &[
    "integer_literal",
    "float_literal",
    "array_expression",
    "unary_expression", // 负数字面量在 tree-sitter-rust 里是 unary_expression(-N)
];
const FONT_LITERAL_KINDS: &[&str] = &["string_literal"];

/// 预编译好的 pattern 集合。**必须**在文件遍历循环外只 `compile` 一次，循环内
/// 传 `&Patterns` 复用——见计划 Global Constraints 的性能约束（spike 已验证
/// 裸 `&str` 传给 `find_all` 会导致每个节点都重新编译一次 pattern）。
pub struct Patterns {
    color_ctors: [Pattern; 3],
    padding: Pattern,
    spacing: Pattern,
    font_method: Pattern,
    font_with_name: Pattern,
    event_handlers: [Pattern; 5],
}

impl Patterns {
    pub fn compile(lang: SupportLang) -> Self {
        Patterns {
            color_ctors: [
                Pattern::new("Color::from_rgb($$$)", lang),
                Pattern::new("Color::from_rgba($$$)", lang),
                Pattern::new("Color::from_rgb8($$$)", lang),
            ],
            padding: Pattern::new("$RECV.padding($X)", lang),
            spacing: Pattern::new("$RECV.spacing($X)", lang),
            font_method: Pattern::new("$RECV.font($X)", lang),
            font_with_name: Pattern::new("Font::with_name($X)", lang),
            event_handlers: [
                Pattern::new("$RECV.on_press($$$)", lang),
                Pattern::new("$RECV.on_enter($$$)", lang),
                Pattern::new("$RECV.on_exit($$$)", lang),
                Pattern::new("$RECV.on_input($$$)", lang),
                Pattern::new("$RECV.on_submit($$$)", lang),
            ],
        }
    }
}

/// 颜色：构造函数调用本身就是发现,不判断参数是不是字面量(项目里 ColorTokens
/// 的正确用法是字段访问,不是再调一次构造函数——见 spec「颜色硬编码」)。
pub fn find_color_findings(
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    patterns: &Patterns,
    file: &Path,
) -> Vec<RawLiteralFinding> {
    patterns
        .color_ctors
        .iter()
        .flat_map(|p| {
            root.find_all(p).map(|m| RawLiteralFinding {
                file: file.to_path_buf(),
                line: m.get_node().start_pos().line() + 1,
                snippet: m.get_node().text().to_string(),
            })
        })
        .collect()
}

fn find_literal_arg_findings(
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    pattern: &Pattern,
    literal_kinds: &[&str],
    file: &Path,
) -> Vec<RawLiteralFinding> {
    root.find_all(pattern)
        .filter_map(|m| {
            let arg = m.get_env().get_match("X")?;
            if literal_kinds.contains(&arg.kind().as_ref()) {
                Some(RawLiteralFinding {
                    file: file.to_path_buf(),
                    line: m.get_node().start_pos().line() + 1,
                    snippet: arg.text().to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// 边距：`.padding($X)`/`.spacing($X)` 的 `$X` 是字面量才算一次发现(变量/
/// 表达式引用不算——见 spec「边距硬编码」)。
pub fn find_spacing_findings(
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    patterns: &Patterns,
    file: &Path,
) -> Vec<RawLiteralFinding> {
    let mut out = find_literal_arg_findings(root, &patterns.padding, SPACING_LITERAL_KINDS, file);
    out.extend(find_literal_arg_findings(
        root,
        &patterns.spacing,
        SPACING_LITERAL_KINDS,
        file,
    ));
    out
}

/// 字体：`.font($X)`/`Font::with_name($X)` 的 `$X` 是裸字符串字面量才算一次
/// 发现(引用具名常量,如 `Font::with_name(CODE_FONT_FAMILY)`,不算——见 spec
/// 「字体硬编码」)。
pub fn find_font_findings(
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    patterns: &Patterns,
    file: &Path,
) -> Vec<RawLiteralFinding> {
    let mut out =
        find_literal_arg_findings(root, &patterns.font_method, FONT_LITERAL_KINDS, file);
    out.extend(find_literal_arg_findings(
        root,
        &patterns.font_with_name,
        FONT_LITERAL_KINDS,
        file,
    ));
    out
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests
```

Expected: PASS，Task 1 + Task 2 全部测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-codehealth/src/ui_metrics.rs
git commit -m "feat(codehealth): 颜色/边距/字体硬编码检测(预编译 Pattern)"
```

---

### Task 3: 组件树嵌套深度（文本扫描）+ 事件回调密度

**Files:**
- Modify: `crates/dozer-codehealth/src/ui_metrics.rs`

**Interfaces:**
- Consumes: `Patterns`（Task 2）。
- Produces（供 Task 5 使用）：
  - `pub fn widget_nesting_depth(fn_source: &str) -> usize`
  - `pub fn event_handler_count(fn_node: &Node<'_, impl Doc>, patterns: &Patterns) -> usize`

- [ ] **Step 1: 写失败测试**

追加到 `ui_metrics.rs` 测试模块：

```rust
    #[test]
    fn widget_nesting_depth_flat_is_one() {
        assert_eq!(widget_nesting_depth("row![text(\"a\")]"), 1);
    }

    #[test]
    fn widget_nesting_depth_counts_nested_row_in_column() {
        assert_eq!(widget_nesting_depth("column![row![text(\"a\")]]"), 2);
    }

    #[test]
    fn widget_nesting_depth_counts_three_levels() {
        assert_eq!(
            widget_nesting_depth("column![row![column![text(\"a\")]]]"),
            3
        );
    }

    #[test]
    fn widget_nesting_depth_siblings_do_not_add_up() {
        // 两个平级的 row!,不是嵌套,深度还是 2(column 包一层 row)。
        assert_eq!(
            widget_nesting_depth("column![row![text(\"a\")], row![text(\"b\")]]"),
            2
        );
    }

    #[test]
    fn widget_nesting_depth_no_widget_macro_is_zero() {
        assert_eq!(widget_nesting_depth("fn f() { let x = 1; }"), 0);
    }

    fn first_function<'r, D: ast_grep_core::Doc>(
        root: &'r ast_grep_core::Node<'r, D>,
    ) -> ast_grep_core::Node<'r, D> {
        root.dfs()
            .find(|n| n.kind() == "function_item")
            .expect("at least one function_item in fixture source")
    }

    #[test]
    fn event_handler_count_sums_all_handler_kinds() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse(
            "fn f() { btn.on_press(Msg::A).into(); area.on_enter(Msg::B); }",
        );
        let f = first_function(&root.root());
        assert_eq!(event_handler_count(&f, &patterns), 2);
    }

    #[test]
    fn event_handler_count_zero_when_no_handlers() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse("fn f() { let x = 1; }");
        let f = first_function(&root.root());
        assert_eq!(event_handler_count(&f, &patterns), 0);
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests -- widget_nesting_depth event_handler_count
```

Expected: 编译失败（两个函数尚未定义）。

- [ ] **Step 3: 实现**

追加到 `ui_metrics.rs`：

```rust
/// 组件树嵌套深度:对函数源码文本做 `row![`/`column![` 方括号深度扫描的
/// 迷你状态机,不用 AST(tree-sitter-rust 不会把嵌套在另一个宏参数里的宏
/// 调用识别成独立节点,spike 已验证技术不可行——见 spec「组件树嵌套深度」)。
/// 已知局限:字符串/注释里偶然出现的 "row![" / "column![" 文本会被误判,
/// 同函数级 complexity_signal 一样是"粗代理,不追求精确"的定位,可接受。
pub fn widget_nesting_depth(fn_source: &str) -> usize {
    let bytes = fn_source.as_bytes();
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    // 栈记录每一层"[" 是不是由 row!/column! 触发的(true)还是普通的 "["
    // (数组字面量等,false,不计入 widget 深度但仍要正确配对好方括号)。
    let mut stack: Vec<bool> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let is_widget = starts_with_widget_macro(&fn_source[..i]);
            if is_widget {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
            stack.push(is_widget);
            i += 1;
        } else if bytes[i] == b']' {
            if let Some(was_widget) = stack.pop()
                && was_widget
            {
                depth = depth.saturating_sub(1);
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    max_depth
}

/// `text_before_bracket` 是从函数源码开头到(不含)当前 "[" 的切片,检查它是不是
/// 紧接着以 "row!" 或 "column!" 结尾(允许前面有空白/换行,同真实代码风格)。
fn starts_with_widget_macro(text_before_bracket: &str) -> bool {
    let trimmed = text_before_bracket.trim_end();
    trimmed.ends_with("row!") || trimmed.ends_with("column!")
}

/// 事件回调密度:`.on_press`/`.on_enter`/`.on_exit`/`.on_input`/`.on_submit`
/// 方法调用总数,复用 Task 2 已验证的 pattern 匹配技术。
pub fn event_handler_count(
    fn_node: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    patterns: &Patterns,
) -> usize {
    patterns
        .event_handlers
        .iter()
        .map(|p| fn_node.find_all(p).count())
        .sum()
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests
```

Expected: PASS，Task 1-3 全部测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-codehealth/src/ui_metrics.rs
git commit -m "feat(codehealth): 组件树嵌套深度(文本扫描)+ 事件回调密度检测"
```

---

### Task 4: 组件化重复结构（结构指纹去重）

**Files:**
- Modify: `crates/dozer-codehealth/src/ui_metrics.rs`

**Interfaces:**
- Consumes: 无新外部依赖。
- Produces（供 Task 6 使用）：
  - `pub fn find_duplicate_clusters(root: &Node<'_, impl Doc>, file: &Path, registry: &mut HashMap<String, Vec<(PathBuf, usize)>>)`（把发现累加进调用方持有的全局 registry，跨文件聚合——签名设计成"追加进传入的 map"而不是"每个文件返回自己的 Vec"，因为去重判定本来就需要跨全部文件比较，`scan_project` 循环里逐文件调用累加，循环结束后再一次性转成 `Vec<DuplicateCluster>`，同 Task 6）
  - `pub fn clusters_from_registry(registry: HashMap<String, Vec<(PathBuf, usize)>>) -> Vec<DuplicateCluster>`（过滤 `occurrences.len() >= 3`，按出现次数降序排列）

- [ ] **Step 1: 写失败测试**

追加到 `ui_metrics.rs` 测试模块（需要 `use std::collections::HashMap;`）：

```rust
    #[test]
    fn find_duplicate_clusters_groups_structurally_identical_widgets() {
        let mut registry = HashMap::new();
        let srcs = [
            "fn a() { row![text(\"x\"), text(\"y\")] }",
            "fn b() { row![text(\"p\"), text(\"q\")] }",
            "fn c() { row![text(\"m\"), text(\"n\")] }",
        ];
        for (i, src) in srcs.iter().enumerate() {
            let root = parse(src);
            find_duplicate_clusters(&root.root(), Path::new(&format!("f{i}.rs")), &mut registry);
        }
        let clusters = clusters_from_registry(registry);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].occurrences.len(), 3);
    }

    #[test]
    fn find_duplicate_clusters_ignores_structurally_different_widgets() {
        let mut registry = HashMap::new();
        let srcs = [
            "fn a() { row![text(\"x\")] }",
            "fn b() { row![text(\"x\"), text(\"y\"), text(\"z\")] }",
        ];
        for (i, src) in srcs.iter().enumerate() {
            let root = parse(src);
            find_duplicate_clusters(&root.root(), Path::new(&format!("f{i}.rs")), &mut registry);
        }
        // 两个结构不同(子元素数量不一样),都只出现 1 次,不构成 >=3 的簇。
        assert!(clusters_from_registry(registry).is_empty());
    }

    #[test]
    fn find_duplicate_clusters_below_threshold_not_included() {
        let mut registry = HashMap::new();
        let srcs = ["fn a() { row![text(\"x\")] }", "fn b() { row![text(\"y\")] }"];
        for (i, src) in srcs.iter().enumerate() {
            let root = parse(src);
            find_duplicate_clusters(&root.root(), Path::new(&format!("f{i}.rs")), &mut registry);
        }
        // 只出现 2 次,< 3 门槛,不成簇。
        assert!(clusters_from_registry(registry).is_empty());
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests -- find_duplicate_clusters
```

Expected: 编译失败（`find_duplicate_clusters`/`clusters_from_registry` 尚未定义）。

- [ ] **Step 3: 实现**

追加到 `ui_metrics.rs`（`use` 段补 `std::collections::HashMap`）：

```rust
const WIDGET_MACRO_NAMES: &[&str] = &["row", "column"];

/// 结构指纹:子树 dfs 只拼接 `node.kind()`,丢弃字面量文本——两个 widget
/// 构造表达式指纹相同说明"形状"一样,即使传的文字/数字不同(见 spec
/// 「组件化重复结构」)。
fn structural_fingerprint<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> String {
    let mut out = String::new();
    fn walk<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>, out: &mut String) {
        out.push('(');
        out.push_str(node.kind().as_ref());
        for c in node.children() {
            walk(&c, out);
        }
        out.push(')');
    }
    walk(node, &mut out);
    out
}

/// 只找**顶层**(不嵌套在另一个宏参数里的)`row!`/`column!` 调用——命中就
/// 不下钻,避免一个大结构和它内部的子结构互相污染彼此的计数。
fn top_level_widget_macros<'t, D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'t, D>,
) -> Vec<ast_grep_core::Node<'t, D>> {
    let mut out = Vec::new();
    fn walk<'t, D: ast_grep_core::Doc>(
        node: &ast_grep_core::Node<'t, D>,
        out: &mut Vec<ast_grep_core::Node<'t, D>>,
    ) {
        let is_target = node.kind() == "macro_invocation"
            && node
                .field("macro")
                .map(|m| WIDGET_MACRO_NAMES.contains(&m.text().as_ref()))
                .unwrap_or(false);
        if is_target {
            out.push(node.clone());
            return;
        }
        for c in node.children() {
            walk(&c, out);
        }
    }
    walk(node, &mut out);
    out
}

/// 把一个文件里顶层 widget 构造的结构指纹累加进跨文件共享的 `registry`。
/// 调用方(`scan_project`)负责在文件循环里逐个调用、循环结束后统一转成
/// `Vec<DuplicateCluster>`(见 `clusters_from_registry`),因为去重判定本来
/// 就需要跨全部文件比较。
pub fn find_duplicate_clusters(
    root: &ast_grep_core::Node<'_, impl ast_grep_core::Doc>,
    file: &Path,
    registry: &mut HashMap<String, Vec<(PathBuf, usize)>>,
) {
    for w in top_level_widget_macros(root) {
        let fp = structural_fingerprint(&w);
        registry
            .entry(fp)
            .or_default()
            .push((file.to_path_buf(), w.start_pos().line() + 1));
    }
}

/// 过滤出现次数 `>= 3` 的指纹组成簇,按出现次数降序排列(同 spec「组件化
/// 重复结构」的簇判定门槛)。
pub fn clusters_from_registry(
    registry: HashMap<String, Vec<(PathBuf, usize)>>,
) -> Vec<DuplicateCluster> {
    let mut clusters: Vec<DuplicateCluster> = registry
        .into_iter()
        .filter(|(_, locs)| locs.len() >= 3)
        .map(|(fp, occurrences)| DuplicateCluster {
            node_count: fp.len(),
            occurrences,
        })
        .collect();
    clusters.sort_by_key(|c| std::cmp::Reverse(c.occurrences.len()));
    clusters
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-codehealth --lib ui_metrics::tests
```

Expected: PASS，Task 1-4 全部测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-codehealth/src/ui_metrics.rs
git commit -m "feat(codehealth): 组件化重复结构检测(结构指纹去重)"
```

---

### Task 5: `FunctionMetric` 新增两个字段 + 修复受影响调用点

**Files:**
- Modify: `crates/dozer-codehealth/src/function_metric.rs`
- Modify: `crates/dozer-codehealth/src/report.rs:145-162`（`file_metric_flags_on_critical_function` 测试里的 `FunctionMetric` 字面量构造）
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs:314-324`（`metric()` 测试 helper）

**Interfaces:**
- Consumes: `ui_metrics::{Patterns, widget_nesting_depth, event_handler_count}`（Task 2/3）。
- Produces（供 Task 6/7 使用）：`FunctionMetric` 新增 `pub widget_nesting_depth: usize`、`pub event_handler_count: usize` 两个字段；`pub fn functions_in_source(src: &str, file: &Path, patterns: &ui_metrics::Patterns) -> Vec<FunctionMetric>` 签名变更（新增 `patterns` 参数）。

- [ ] **Step 1: 写失败测试**

修改 `crates/dozer-codehealth/src/function_metric.rs` 里现有的 4 个 `functions_in_source` 测试调用点，加 `patterns` 参数；并新增两个断言新字段的测试：

```rust
    #[test]
    fn functions_in_source_extracts_name_and_loc() {
        let src = "fn foo() {\n    let x = 1;\n    x\n}\n";
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        let metrics = functions_in_source(src, Path::new("a.rs"), &patterns);
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
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        let metrics = functions_in_source(src, Path::new("b.rs"), &patterns);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].complexity_signal, 3);
    }

    #[test]
    fn functions_in_source_handles_multiple_functions() {
        let src = "fn one() {}\nfn two() {}\n";
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        let metrics = functions_in_source(src, Path::new("c.rs"), &patterns);
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "one");
        assert_eq!(metrics[1].name, "two");
    }

    #[test]
    fn functions_in_source_empty_file_returns_empty() {
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        assert!(functions_in_source("", Path::new("empty.rs"), &patterns).is_empty());
    }

    #[test]
    fn functions_in_source_computes_widget_nesting_depth() {
        let src = "fn view() -> Element {\n    column![row![text(\"a\")]]\n}\n";
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        let metrics = functions_in_source(src, Path::new("d.rs"), &patterns);
        assert_eq!(metrics[0].widget_nesting_depth, 2);
    }

    #[test]
    fn functions_in_source_computes_event_handler_count() {
        let src = "fn view() -> Element {\n    btn.on_press(Msg::A)\n}\n";
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        let metrics = functions_in_source(src, Path::new("e.rs"), &patterns);
        assert_eq!(metrics[0].event_handler_count, 1);
    }
```

同时修改 `crates/dozer-codehealth/src/report.rs` 里 `file_metric_flags_on_critical_function` 测试（约第 149-157 行）的 `FunctionMetric` 字面量构造，加两个新字段：

```rust
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
            widget_nesting_depth: 0,
            event_handler_count: 0,
        };
        let m = file_metric(Path::new("a.rs"), 100, &[f]);
        assert_eq!(m.critical_functions, 1);
        assert_eq!(m.watch_functions, 0);
        assert!(m.flagged);
    }
```

同时修改 `crates/dozer-app/src/extensions/codehealth/view.rs` 里 `metric()` 测试 helper（约第 314-324 行），加两个新字段（默认 0，这批测试不关心 UI 指标）：

```rust
    fn metric(name: &str, severity: Severity, complexity_signal: usize) -> FunctionMetric {
        FunctionMetric {
            name: name.to_string(),
            file: std::path::PathBuf::from("a.rs"),
            start_line: 1,
            end_line: 2,
            loc: 2,
            complexity_signal,
            severity,
            widget_nesting_depth: 0,
            event_handler_count: 0,
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-codehealth --lib 2>&1 | tail -40
```

Expected: 编译失败——`FunctionMetric` 缺 `widget_nesting_depth`/`event_handler_count` 字段、`functions_in_source` 参数数量不匹配。

- [ ] **Step 3: 加字段 + 改签名 + 接线**

修改 `crates/dozer-codehealth/src/function_metric.rs` 的 `FunctionMetric` 定义（原 13-22 行）：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionMetric {
    pub name: String,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub loc: usize,
    pub complexity_signal: usize,
    pub severity: Severity,
    pub widget_nesting_depth: usize,
    pub event_handler_count: usize,
}
```

修改 `functions_in_source`（原 50-74 行）签名和实现：

```rust
pub fn functions_in_source(
    src: &str,
    file: &Path,
    patterns: &crate::ui_metrics::Patterns,
) -> Vec<FunctionMetric> {
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
            let fn_source = f.text();
            FunctionMetric {
                name,
                file: file.to_path_buf(),
                start_line,
                end_line,
                loc: end_line - start_line + 1,
                complexity_signal,
                severity: severity_for(complexity_signal),
                widget_nesting_depth: crate::ui_metrics::widget_nesting_depth(&fn_source),
                event_handler_count: crate::ui_metrics::event_handler_count(&f, patterns),
            }
        })
        .collect()
}
```

（`f.text()` 返回函数节点的完整源码切片，`ast_grep_core::Node::text()` 已经在别处用过——见本文件 `problem_row`/`find_color_findings` 等既有用法，不是新增 API。）

修改 `crates/dozer-codehealth/src/report.rs::scan_project`（原 94-130 行）里调用 `functions_in_source` 的那一行，改成传 `patterns`（在文件循环外编译一次，同 Global Constraints 的性能约束）：

```rust
pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport> {
    let mut files = Vec::new();
    if root.is_dir() {
        collect_rs_files(root, &mut files)?;
    }

    let patterns = crate::ui_metrics::Patterns::compile(ast_grep_language::SupportLang::Rust);

    let mut all_functions = Vec::new();
    let mut total_loc = 0usize;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        total_loc += src.lines().count();
        all_functions.extend(crate::function_metric::functions_in_source(
            &src, path, &patterns,
        ));
    }
    // ...(其余不变,Task 6 会在这个函数里继续扩展)
```

（`report.rs` 顶部需要加 `use ast_grep_language::SupportLang;`——如果还没有的话；`function_metric.rs` 顶部已经 `use ast_grep_language::{LanguageExt, SupportLang};`，不需要再改。）

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-codehealth --lib
cargo build -p dozer-app 2>&1 | tail -20
```

Expected: `dozer-codehealth` 全部测试 PASS；`dozer-app` 因为 `view.rs`/`mod.rs` 里还有 `ProjectReport` 字面量构造缺字段（Task 6 才加），此时预期会编译失败，报"missing fields"——**这是预期的中间状态，Task 6 会修**，不用现在管。只要确认失败原因是"`ProjectReport` 缺字段"而不是别的（比如不是因为 `FunctionMetric`/`functions_in_source` 相关的错误——那两处已经在这个 Task 修完，`dozer-app` 里没有直接构造 `FunctionMetric` 字面量以外的调用点，`view.rs::metric()` 已经在 Step 1 改过了）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-codehealth/src/function_metric.rs crates/dozer-codehealth/src/report.rs crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "feat(codehealth): FunctionMetric 新增嵌套深度/事件回调字段"
```

---

### Task 6: `ProjectReport` 新增 UI 指标字段 + `scan_project` 整合 + 修复 `dozer-app` 编译

**Files:**
- Modify: `crates/dozer-codehealth/src/report.rs`
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs:99-109`（`sample_report()` helper）
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs:274-290,326-339`（两个 `sample_report`/`report_with_functions` helper）

**Interfaces:**
- Consumes: `ui_metrics::{RawLiteralFinding, DuplicateCluster, find_color_findings, find_spacing_findings, find_font_findings, find_duplicate_clusters, clusters_from_registry, color_tier, spacing_tier, font_tier, cluster_tier, nesting_depth_tier, event_handler_tier}`（Task 1-4）。
- Produces（供 Task 7 使用）：`ProjectReport` 新增十个字段（见 Step 3）。

- [ ] **Step 1: 写失败测试**

追加到 `crates/dozer-codehealth/src/report.rs` 的 `#[cfg(test)] mod scan_tests`：

```rust
    #[test]
    fn scan_project_collects_color_findings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.rs"),
            "fn f() { let c = Color::from_rgb(0.1, 0.2, 0.3); }",
        )
        .unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.color_findings.len(), 1);
        assert_eq!(report.color_tier, HealthTier::Watch);
    }

    #[test]
    fn scan_project_computes_distinct_spacing_values() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.rs"),
            "fn f() { col.padding(8).padding(8).spacing(16); }",
        )
        .unwrap();
        let report = scan_project(dir.path()).unwrap();
        // 8 出现两次、16 一次,distinct 应该是 2。
        assert_eq!(report.distinct_spacing_values, 2);
    }

    #[test]
    fn scan_project_finds_duplicate_clusters_across_files() {
        let dir = tempfile::tempdir().unwrap();
        for (i, body) in ["x", "y", "z"].iter().enumerate() {
            fs::write(
                dir.path().join(format!("f{i}.rs")),
                format!("fn f() {{ row![text(\"{body}\")] }}"),
            )
            .unwrap();
        }
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.duplicate_clusters.len(), 1);
        assert_eq!(report.duplicate_clusters[0].occurrences.len(), 3);
    }

    #[test]
    fn scan_project_ui_tier_takes_worst_of_six_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        // 16 处颜色硬编码 -> color_tier = Critical,其余维度都是 Healthy,
        // ui_tier 应该跟着变成 Critical。
        let calls: String = (0..16)
            .map(|i| format!("let _c{i} = Color::from_rgb(0.1, 0.2, 0.3);\n"))
            .collect();
        fs::write(dir.path().join("a.rs"), format!("fn f() {{\n{calls}}}\n")).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.color_tier, HealthTier::Critical);
        assert_eq!(report.ui_tier, HealthTier::Critical);
    }

    #[test]
    fn scan_project_empty_dir_ui_fields_all_zero() {
        let dir = tempfile::tempdir().unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert!(report.color_findings.is_empty());
        assert!(report.spacing_findings.is_empty());
        assert!(report.font_findings.is_empty());
        assert!(report.duplicate_clusters.is_empty());
        assert_eq!(report.distinct_color_values, 0);
        assert_eq!(report.distinct_spacing_values, 0);
        assert_eq!(report.ui_tier, HealthTier::Healthy);
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-codehealth --lib scan_tests -- scan_project_collects_color_findings scan_project_computes_distinct_spacing_values scan_project_finds_duplicate_clusters_across_files scan_project_ui_tier_takes_worst_of_six_dimensions scan_project_empty_dir_ui_fields_all_zero
```

Expected: 编译失败（`ProjectReport` 还没有这些字段）。

- [ ] **Step 3: 扩展 `ProjectReport` + `scan_project`**

修改 `crates/dozer-codehealth/src/report.rs` 的 `ProjectReport` 定义（原 40-49 行）：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReport {
    pub total_loc: usize,
    pub total_functions: usize,
    pub critical_functions: usize,
    pub scale_tier: HealthTier,
    pub density_tier: HealthTier,
    pub overall_tier: HealthTier,
    pub functions: Vec<FunctionMetric>,

    pub color_findings: Vec<crate::ui_metrics::RawLiteralFinding>,
    pub color_tier: HealthTier,
    pub spacing_findings: Vec<crate::ui_metrics::RawLiteralFinding>,
    pub spacing_tier: HealthTier,
    pub font_findings: Vec<crate::ui_metrics::RawLiteralFinding>,
    pub font_tier: HealthTier,
    pub distinct_color_values: usize,
    pub distinct_spacing_values: usize,
    pub duplicate_clusters: Vec<crate::ui_metrics::DuplicateCluster>,
    pub duplicate_cluster_tier: HealthTier,
    pub nesting_depth_tier: HealthTier,
    pub event_handler_tier: HealthTier,
    pub ui_tier: HealthTier,
}
```

修改 `scan_project`（Task 5 已经改过前半段，这里接着改完整函数体，替换掉原来 94-130 行剩余部分）：

在 `report.rs` 顶部 `use` 段加上 `use ast_grep_language::{LanguageExt, SupportLang};`（`function_metric.rs` 已经这样 import 过，这里是同一套标准调用方式，不用 UFCS 全限定写法）：

```rust
pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport> {
    use crate::ui_metrics::{
        self, Patterns, clusters_from_registry, color_tier, event_handler_tier, font_tier,
        nesting_depth_tier, spacing_tier,
    };
    use std::collections::HashMap;

    let mut files = Vec::new();
    if root.is_dir() {
        collect_rs_files(root, &mut files)?;
    }

    let patterns = Patterns::compile(SupportLang::Rust);

    let mut all_functions = Vec::new();
    let mut total_loc = 0usize;
    let mut color_findings = Vec::new();
    let mut spacing_findings = Vec::new();
    let mut font_findings = Vec::new();
    let mut duplicate_registry: HashMap<String, Vec<(PathBuf, usize)>> = HashMap::new();

    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        total_loc += src.lines().count();
        let ast = SupportLang::Rust.ast_grep(&src);
        let root_node = ast.root();
        // 这里和 `functions_in_source` 内部各自独立 `ast_grep(&src)` 解析了
        // 一次同一份源码——重复解析、不是共享同一棵树。已知的小效率损耗:
        // tree-sitter 解析本身很快(spike 瓶颈是 pattern 重复编译,不是解析),
        // 全仓库量级下多一次解析仍然在个位数到十几秒可接受范围(Task 8 会
        // 实测确认),不为省这一次解析去改 `functions_in_source` 签名接收
        // 外部传入的 root 节点,增加不必要的复杂度。

        all_functions.extend(crate::function_metric::functions_in_source(
            &src, path, &patterns,
        ));
        color_findings.extend(ui_metrics::find_color_findings(&root_node, &patterns, path));
        spacing_findings.extend(ui_metrics::find_spacing_findings(
            &root_node, &patterns, path,
        ));
        font_findings.extend(ui_metrics::find_font_findings(&root_node, &patterns, path));
        ui_metrics::find_duplicate_clusters(&root_node, path, &mut duplicate_registry);
    }

    let total_functions = all_functions.len();
    let critical_functions = all_functions
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let scale = scale_tier(total_loc);
    let density = density_tier(critical_functions, total_functions);
    let mut functions = all_functions;
    functions.sort_by_key(|f| std::cmp::Reverse(f.severity_rank()));

    let distinct_color_values: usize = color_findings
        .iter()
        .map(|f| f.snippet.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let distinct_spacing_values: usize = spacing_findings
        .iter()
        .map(|f| f.snippet.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    let duplicate_clusters = clusters_from_registry(duplicate_registry);
    let color_t = color_tier(color_findings.len());
    let spacing_t = spacing_tier(spacing_findings.len());
    let font_t = font_tier(font_findings.len());
    let duplicate_t = duplicate_clusters
        .first()
        .map(|c| crate::ui_metrics::cluster_tier(c.occurrences.len()))
        .unwrap_or(HealthTier::Healthy);
    let nesting_t = functions
        .iter()
        .map(|f| nesting_depth_tier(f.widget_nesting_depth))
        .max()
        .unwrap_or(HealthTier::Healthy);
    let handler_t = functions
        .iter()
        .map(|f| event_handler_tier(f.event_handler_count))
        .max()
        .unwrap_or(HealthTier::Healthy);
    let ui_tier = [color_t, spacing_t, font_t, duplicate_t, nesting_t, handler_t]
        .into_iter()
        .max()
        .unwrap();

    Ok(ProjectReport {
        total_loc,
        total_functions,
        critical_functions,
        scale_tier: scale,
        density_tier: density,
        overall_tier: scale.max(density),
        functions,
        color_findings,
        color_tier: color_t,
        spacing_findings,
        spacing_tier: spacing_t,
        font_findings,
        font_tier: font_t,
        distinct_color_values,
        distinct_spacing_values,
        duplicate_clusters,
        duplicate_cluster_tier: duplicate_t,
        nesting_depth_tier: nesting_t,
        event_handler_tier: handler_t,
        ui_tier,
    })
}
```

`duplicate_clusters` 已经按出现次数降序排过（`clusters_from_registry` 内部排序），`.first()` 拿到的就是"最严重的簇"，用它的出现次数算 `duplicate_cluster_tier`，同 `nesting_t`/`handler_t`"取全部函数里最严重那个"的处理方式一致。

- [ ] **Step 4: 修复 `dozer-app` 里的 `ProjectReport` 字面量构造**

修改 `crates/dozer-app/src/extensions/codehealth/mod.rs` 的 `sample_report()`（原 99-109 行）：

```rust
    fn sample_report() -> ProjectReport {
        ProjectReport {
            total_loc: 100,
            total_functions: 5,
            critical_functions: 1,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Watch,
            overall_tier: HealthTier::Watch,
            functions: vec![],
            color_findings: vec![],
            color_tier: HealthTier::Healthy,
            spacing_findings: vec![],
            spacing_tier: HealthTier::Healthy,
            font_findings: vec![],
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: vec![],
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }
```

修改 `crates/dozer-app/src/extensions/codehealth/view.rs` 的 `sample_report()`（原 274-290 行）和 `report_with_functions()`（原 326-339 行），同样补齐新增的十个字段（`sample_report` 的两个调用点都传 `HealthTier::Healthy`/空 `Vec`/`0` 默认值，不影响原有测试断言的字段；`report_with_functions` 同样处理）：

```rust
    fn sample_report(
        total_loc: usize,
        scale_tier: HealthTier,
        critical_functions: usize,
        total_functions: usize,
        density_tier: HealthTier,
    ) -> ProjectReport {
        ProjectReport {
            total_loc,
            total_functions,
            critical_functions,
            scale_tier,
            density_tier,
            overall_tier: scale_tier.max(density_tier),
            functions: Vec::new(),
            color_findings: Vec::new(),
            color_tier: HealthTier::Healthy,
            spacing_findings: Vec::new(),
            spacing_tier: HealthTier::Healthy,
            font_findings: Vec::new(),
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: Vec::new(),
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }
```

```rust
    fn report_with_functions(functions: Vec<FunctionMetric>) -> ProjectReport {
        ProjectReport {
            total_loc: 0,
            total_functions: functions.len(),
            critical_functions: functions
                .iter()
                .filter(|f| f.severity == Severity::Critical)
                .count(),
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Healthy,
            overall_tier: HealthTier::Healthy,
            functions,
            color_findings: Vec::new(),
            color_tier: HealthTier::Healthy,
            spacing_findings: Vec::new(),
            spacing_tier: HealthTier::Healthy,
            font_findings: Vec::new(),
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: Vec::new(),
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }
```

- [ ] **Step 5: 运行测试确认通过**

```bash
cargo test -p dozer-codehealth --lib
cargo test -p dozer-app --lib codehealth
cargo build --workspace 2>&1 | tail -30
```

Expected: 全部 PASS，`cargo build --workspace` 编译通过（回到全绿状态）。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-codehealth/src/report.rs crates/dozer-app/src/extensions/codehealth/mod.rs crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "feat(codehealth): ProjectReport 整合六维度 UI 指标,scan_project 接线"
```

---

### Task 7: `dozer-app` 面板渲染——"UI 一致性"分类分区

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs`

**Interfaces:**
- Consumes: `ProjectReport` 的十个新字段（Task 6）；现有 `tier_color`/`tier_label`（`view.rs:13-27`，不改）；现有 `Message::OpenLocation`（不改）。
- Produces: `content_pane` 在 `problem_list(report)` 之后追加渲染，签名不变。

- [ ] **Step 1: 实现渲染函数**

在 `view.rs` 的 `problem_list` 函数（原 185-204 行）之后、`error_banner` 之前插入（代码后附了 `tokens` 传值方式和生命周期标注的说明）：

```rust
fn raw_literal_findings_section(
    title: &str,
    findings: &[dozer_codehealth::RawLiteralFinding],
    tier: HealthTier,
    distinct_count: Option<usize>,
    tokens: ColorTokens,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut header_text = format!("{title}：{}（{} 处）", tier_label(tier), findings.len());
    if let Some(n) = distinct_count {
        header_text.push_str(&format!("，{n} 种不同取值"));
    }
    let mut col = column![text(header_text).size(13).color(tier_color(tier, &tokens))].spacing(4);
    for f in findings {
        let line_text = format!("{}:{}  {}", f.file.display(), f.line, f.snippet);
        let content = text(line_text).size(11).color(tokens.dim);
        col = col.push(
            mouse_area(content)
                .on_press(Message::OpenLocation(f.file.clone(), f.line))
                .into(),
        );
    }
    col.padding([4, 8]).into()
}

fn nesting_depth_section(
    report: &ProjectReport,
    tokens: ColorTokens,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut offenders: Vec<&FunctionMetric> = report
        .functions
        .iter()
        .filter(|f| dozer_codehealth::nesting_depth_tier(f.widget_nesting_depth) != HealthTier::Healthy)
        .collect();
    offenders.sort_by_key(|f| std::cmp::Reverse(f.widget_nesting_depth));
    let header = format!(
        "组件树嵌套深度：{}（{} 个函数超标）",
        tier_label(report.nesting_depth_tier),
        offenders.len()
    );
    let mut col =
        column![text(header).size(13).color(tier_color(report.nesting_depth_tier, &tokens))].spacing(4);
    for f in offenders {
        let line_text = format!(
            "{}:{}  {}  depth={}",
            f.file.display(),
            f.start_line,
            f.name,
            f.widget_nesting_depth
        );
        let content = text(line_text).size(11).color(tokens.dim);
        col = col.push(
            mouse_area(content)
                .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
                .into(),
        );
    }
    col.padding([4, 8]).into()
}

fn event_handler_section(
    report: &ProjectReport,
    tokens: ColorTokens,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut offenders: Vec<&FunctionMetric> = report
        .functions
        .iter()
        .filter(|f| dozer_codehealth::event_handler_tier(f.event_handler_count) != HealthTier::Healthy)
        .collect();
    offenders.sort_by_key(|f| std::cmp::Reverse(f.event_handler_count));
    let header = format!(
        "事件回调密度：{}（{} 个函数超标）",
        tier_label(report.event_handler_tier),
        offenders.len()
    );
    let mut col =
        column![text(header).size(13).color(tier_color(report.event_handler_tier, &tokens))].spacing(4);
    for f in offenders {
        let line_text = format!(
            "{}:{}  {}  回调数={}",
            f.file.display(),
            f.start_line,
            f.name,
            f.event_handler_count
        );
        let content = text(line_text).size(11).color(tokens.dim);
        col = col.push(
            mouse_area(content)
                .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
                .into(),
        );
    }
    col.padding([4, 8]).into()
}

fn duplicate_clusters_section(
    report: &ProjectReport,
    tokens: ColorTokens,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = format!(
        "组件化重复结构：{}（{} 组）",
        tier_label(report.duplicate_cluster_tier),
        report.duplicate_clusters.len()
    );
    let mut col = column![text(header)
        .size(13)
        .color(tier_color(report.duplicate_cluster_tier, &tokens))]
    .spacing(4);
    for cluster in &report.duplicate_clusters {
        let tier = dozer_codehealth::cluster_tier(cluster.occurrences.len());
        let summary = format!("出现 {} 次", cluster.occurrences.len());
        col = col.push(text(summary).size(12).color(tier_color(tier, &tokens)));
        for (file, line) in &cluster.occurrences {
            let line_text = format!("{}:{}", file.display(), line);
            let content = text(line_text).size(11).color(tokens.dim);
            col = col.push(
                mouse_area(content)
                    .on_press(Message::OpenLocation(file.clone(), *line))
                    .into(),
            );
        }
    }
    col.padding([4, 8]).into()
}

fn ui_consistency_section(
    report: &ProjectReport,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    column![
        text("UI 一致性").size(16).color(tokens.body),
        raw_literal_findings_section(
            "颜色硬编码",
            &report.color_findings,
            report.color_tier,
            Some(report.distinct_color_values),
            tokens,
        ),
        raw_literal_findings_section(
            "边距硬编码",
            &report.spacing_findings,
            report.spacing_tier,
            Some(report.distinct_spacing_values),
            tokens,
        ),
        raw_literal_findings_section(
            "字体硬编码",
            &report.font_findings,
            report.font_tier,
            None,
            tokens,
        ),
        nesting_depth_section(report, tokens),
        event_handler_section(report, tokens),
        duplicate_clusters_section(report, tokens),
    ]
    .spacing(12)
    .padding(16)
    .into()
}
```

（四个新函数的 `tokens` 参数一律按值传（`ColorTokens: Copy`），不按引用——同现有 `problem_row(f, tokens: ColorTokens)`/`problem_list` 的既有约定（`view.rs:156-158` 已经这样做）；调用 `tier_color(tier, &tokens)` 时按引用传，因为 `tier_color` 自己的既有签名是 `&ColorTokens`，这个函数本任务不改。四个新函数体内只构造 owned 数据（`format!`/`.clone()`/`.to_string()`），不借用任何输入参数，所以返回类型都标 `'static`，`ui_consistency_section` 组合它们时不需要操心生命周期匹配——`'static` 数据在任何更短的生命周期语境里都能直接使用。）

- [ ] **Step 2: 接入 `content_pane`**

修改 `content_pane`（原 216-260 行）末尾，在 `col.push(problem_list(report))` 之后追加：

```rust
    let mut col = column![health_card(report, ws_state.scanned_at_ms())];
    if let Some(err) = error {
        col = col.push(err);
    }
    col = col.push(problem_list(report));
    col = col.push(ui_consistency_section(report));
    container(col).width(width).into()
```

（`ws_state.scanning()` 分支——原 223-238 行——是否需要同步加 `ui_consistency_section`：不需要，扫描中展示上一次结果时保持原样即可，UI 一致性区块不是"扫描中"分支强调的重点，等扫描完成走非 `scanning()` 分支自然会展示最新结果，同现有 `problem_list` 在 scanning 分支里也只是原样复用的处理方式一致。)

- [ ] **Step 3: 编译确认**

```bash
cargo build -p dozer-app 2>&1 | tail -40
```

Expected: 编译成功，无警告。四个新函数都标了 `'static` 返回生命周期——如果编译报生命周期不匹配，说明哪处不小心直接存了 `report`/`findings`/`f` 的引用而不是先 `format!`/`.clone()`/`.to_string()` 转成 owned 数据再放进 widget，按报错定位后改成 owned，不要把返回类型改回 `'_` 绕过（那样 `ui_consistency_section` 的组合逻辑也要跟着变复杂，没必要）。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "feat(codehealth): 面板新增 UI 一致性分类分区(颜色/边距/字体/嵌套深度/回调密度/重复结构)"
```

---

### Task 8: 全量测试 + clippy + 人工验收

**Files:** 无新改动，只验证 Task 1-7 的组合结果。

**Interfaces:** 无（验收任务）。

- [ ] **Step 1: 全量测试**

```bash
cargo test -p dozer-codehealth --lib
cargo test -p dozer-app --lib codehealth
```

Expected: PASS，Task 1-7 全部测试通过。

- [ ] **Step 2: Clippy + fmt 检查**

```bash
cargo clippy --all-targets -p dozer-codehealth -p dozer-app -- -D warnings
cargo fmt --check
```

Expected: 无警告、无格式差异。若 `cargo fmt` 有差异，运行 `cargo fmt`（不带 `--check`）后重新 `git add` 并追加一个 `style: cargo fmt` commit。

- [ ] **Step 3: 真实扫描验证——用 Dozer 自己的代码库核对 spec 里记录的真实数据**

```bash
cargo run -p dozer-app
```

打开代码健康度面板，点"扫描"，核对：

1. "UI 一致性"区块出现在问题列表下方，六个子区块（颜色/边距/字体/组件树嵌套深度/事件回调密度/组件化重复结构）都有渲染，每个子区块标题旁有严重度文字（健康/关注/警戒）+ 对应颜色。
2. 边距硬编码的发现数应该是三位数量级（spec 记录的 spike 数据是 399，实际数字可能因为这次实现细节/仓库增量变化有出入，但量级应该接近，不应该是 0 或个位数——如果差异很大，先检查是不是 `.padding`/`.spacing` 的 pattern 写法或字面量 kind 判断哪里有问题，不要直接改动阈值常量掩盖过去）。
3. 颜色硬编码应该在个位数到十几的量级（spike 记录 11）。
4. 组件化重复结构区块应该至少能看到几个簇，最大的一个簇出现次数是两位数量级（spike 记录 86，位置在 `app/update.rs`）。
5. 点击任意一条发现/超标函数/重复簇位置，确认能跳转到 Files 面板对应代码行（`Message::OpenLocation` 链路复用现有实现，本次没有新改动，但要确认接入新区块后没有被意外破坏）。
6. 整个扫描耗时应该在个位数秒到十几秒量级（spike 在预编译 pattern 后测得全仓库 6.8 秒），**不应该**出现卡死或耗时几分钟——如果明显变慢，检查是不是哪处 `find_all` 不小心传了裸 `&str` 而不是预编译的 `&Pattern`（Global Constraints 里的性能约束）。

- [ ] **Step 4: Commit（若 Step 2 产生了 fmt 差异）**

```bash
git add -A
git commit -m "style(codehealth): cargo fmt"
```

（若 Step 2 无差异，跳过本步骤，不产生空 commit。）
