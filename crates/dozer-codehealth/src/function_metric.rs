use ast_grep_core::Doc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Normal,
    Watch,
    Critical,
}

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

/// spec「函数级」判定表：`> 40` Critical，`16..=40` Watch，`<= 15` Normal。
pub fn severity_for(complexity_signal: usize) -> Severity {
    if complexity_signal > 40 {
        Severity::Critical
    } else if complexity_signal >= 16 {
        Severity::Watch
    } else {
        Severity::Normal
    }
}

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

/// 从**已经解析好的** AST 根节点提取函数级指标。调用方（`scan_project`）负责
/// 只解析一次源码并把根节点传进来，同一棵树同时供结构复杂度与 UI 规则复用
/// （spec 性能约束「同一源文件只解析一次 AST」）。
pub fn functions_in_source<D: Doc>(
    root: &ast_grep_core::Node<'_, D>,
    file: &Path,
    patterns: &crate::ui_metrics::Patterns,
) -> Vec<FunctionMetric> {
    root.dfs()
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

impl FunctionMetric {
    /// 供 `ProjectReport.functions` 按严重度降序排列用，不对外暴露 `Severity`
    /// 的 `Ord`——`Critical` 应排最前。
    pub(crate) fn severity_rank(&self) -> u8 {
        match self.severity {
            Severity::Critical => 2,
            Severity::Watch => 1,
            Severity::Normal => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_language::{LanguageExt, SupportLang};

    fn parse(src: &str) -> ast_grep_core::AstGrep<impl ast_grep_core::Doc> {
        SupportLang::Rust.ast_grep(src)
    }

    fn metrics_of(src: &str, file: &Path) -> Vec<FunctionMetric> {
        let patterns = crate::ui_metrics::Patterns::compile(SupportLang::Rust);
        let root = parse(src);
        functions_in_source(&root.root(), file, &patterns)
    }

    #[test]
    fn severity_boundaries() {
        assert_eq!(severity_for(15), Severity::Normal);
        assert_eq!(severity_for(16), Severity::Watch);
        assert_eq!(severity_for(40), Severity::Watch);
        assert_eq!(severity_for(41), Severity::Critical);
    }

    #[test]
    fn functions_in_source_extracts_name_and_loc() {
        let src = "fn foo() {\n    let x = 1;\n    x\n}\n";
        let metrics = metrics_of(src, Path::new("a.rs"));
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
        let metrics = metrics_of(src, Path::new("b.rs"));
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].complexity_signal, 3);
    }

    #[test]
    fn functions_in_source_handles_multiple_functions() {
        let src = "fn one() {}\nfn two() {}\n";
        let metrics = metrics_of(src, Path::new("c.rs"));
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "one");
        assert_eq!(metrics[1].name, "two");
    }

    #[test]
    fn functions_in_source_empty_file_returns_empty() {
        let metrics = metrics_of("", Path::new("empty.rs"));
        assert!(metrics.is_empty());
    }

    #[test]
    fn functions_in_source_computes_widget_nesting_depth() {
        let src = "fn view() -> Element {\n    column![row![text(\"a\")]]\n}\n";
        let metrics = metrics_of(src, Path::new("d.rs"));
        assert_eq!(metrics[0].widget_nesting_depth, 2);
    }

    #[test]
    fn functions_in_source_computes_event_handler_count() {
        let src = "fn view() -> Element {\n    btn.on_press(Msg::A)\n}\n";
        let metrics = metrics_of(src, Path::new("e.rs"));
        assert_eq!(metrics[0].event_handler_count, 1);
    }
}
