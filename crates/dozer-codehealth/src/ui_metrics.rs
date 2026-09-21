use crate::report::HealthTier;
use ast_grep_core::matcher::Pattern;
use ast_grep_language::SupportLang;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// 结构指纹（`structural_fingerprint` 输出），作为跨扫描稳定身份的签名。
    /// 旧 JSON 无此字段时回落空串（`#[serde(default)]`），不影响旧报告读取。
    #[serde(default)]
    pub signature: String,
}

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
    let mut out = find_literal_arg_findings(root, &patterns.font_method, FONT_LITERAL_KINDS, file);
    out.extend(find_literal_arg_findings(
        root,
        &patterns.font_with_name,
        FONT_LITERAL_KINDS,
        file,
    ));
    out
}

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
            signature: fp,
            occurrences,
        })
        .collect();
    clusters.sort_by_key(|c| std::cmp::Reverse(c.occurrences.len()));
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_language::{LanguageExt, SupportLang};
    use std::collections::HashMap;
    use std::path::Path;

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

    fn first_function<'a, D: ast_grep_core::Doc>(
        root: &ast_grep_core::Node<'a, D>,
    ) -> ast_grep_core::Node<'a, D> {
        root.dfs()
            .find(|n| n.kind() == "function_item")
            .expect("at least one function_item in fixture source")
    }

    #[test]
    fn event_handler_count_sums_all_handler_kinds() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse("fn f() { btn.on_press(Msg::A).into(); area.on_enter(Msg::B); }");
        let root_node = root.root();
        let f = first_function(&root_node);
        assert_eq!(event_handler_count(&f, &patterns), 2);
    }

    #[test]
    fn event_handler_count_zero_when_no_handlers() {
        let patterns = Patterns::compile(SupportLang::Rust);
        let root = parse("fn f() { let x = 1; }");
        let root_node = root.root();
        let f = first_function(&root_node);
        assert_eq!(event_handler_count(&f, &patterns), 0);
    }

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
        let srcs = [
            "fn a() { row![text(\"x\")] }",
            "fn b() { row![text(\"y\")] }",
        ];
        for (i, src) in srcs.iter().enumerate() {
            let root = parse(src);
            find_duplicate_clusters(&root.root(), Path::new(&format!("f{i}.rs")), &mut registry);
        }
        // 只出现 2 次,< 3 门槛,不成簇。
        assert!(clusters_from_registry(registry).is_empty());
    }
}
