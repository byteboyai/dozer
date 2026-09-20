use crate::report::HealthTier;
use ast_grep_core::matcher::Pattern;
use ast_grep_language::SupportLang;
use serde::{Deserialize, Serialize};
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

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_language::{LanguageExt, SupportLang};
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
}
