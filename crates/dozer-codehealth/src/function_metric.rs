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
}
