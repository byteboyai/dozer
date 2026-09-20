//! Spike: 验证 ast-grep-core + ast-grep-language 能否作为「代码分析」面板的地基。
//!
//! 跑法：`cargo run -p spike-code-analysis -- <要扫描的目录，默认 ../../crates>`
//!
//! 验证两件事：
//! 1. 宏观结构提取——每个文件的函数列表 + 粗粒度嵌套复杂度，对应用户想要的
//!    "代码结构化/宏观认识"面板。
//! 2. 规则化模式匹配——用 ast-grep 的 pattern 语法抓 `.unwrap()` 调用，
//!    验证同一套库能否再长出一个独立于 agent 自评的 CodeReview 雏形。

use anyhow::Result;
use ast_grep_language::{LanguageExt, SupportLang};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct FunctionSummary {
    name: String,
    start_line: usize,
    end_line: usize,
    nesting_depth: usize,
}

#[derive(Serialize)]
struct FileSummary {
    path: String,
    loc: usize,
    functions: Vec<FunctionSummary>,
    unwrap_call_sites: Vec<usize>,
}

/// 粗粒度嵌套复杂度：数一遍函数体内控制流节点的种类，不追求精确圈复杂度，
/// 只作为"这个函数值不值得重点看"的排序信号。
fn nesting_depth<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> usize {
    const CONTROL_FLOW_KINDS: &[&str] = &[
        "if_expression",
        "match_expression",
        "for_expression",
        "while_expression",
        "loop_expression",
        "closure_expression",
    ];
    node.dfs()
        .filter(|n| CONTROL_FLOW_KINDS.contains(&n.kind().as_ref()))
        .count()
}

fn analyze_file(path: &Path) -> Result<Option<FileSummary>> {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Ok(None);
    };
    let Ok(lang) = ext.parse::<SupportLang>() else {
        return Ok(None);
    };

    let src = std::fs::read_to_string(path)?;
    let loc = src.lines().count();
    let root = lang.ast_grep(&src);
    let root_node = root.root();

    let functions = root_node
        .dfs()
        .filter(|n| n.kind() == "function_item")
        .map(|f| {
            let name = f
                .field("name")
                .map(|n| n.text().to_string())
                .unwrap_or_else(|| "<anonymous>".to_string());
            FunctionSummary {
                name,
                start_line: f.start_pos().line() + 1,
                end_line: f.end_pos().line() + 1,
                nesting_depth: nesting_depth(&f),
            }
        })
        .collect();

    // 规则化模式匹配demo：抓 `$X.unwrap()`，验证 CodeReview 类规则同样能建在
    // 这套库上，而不需要额外接入 Semgrep/CodeQL 这类外部进程。
    let unwrap_call_sites = root_node
        .find_all("$X.unwrap()")
        .map(|m| m.get_node().start_pos().line() + 1)
        .collect();

    Ok(Some(FileSummary {
        path: path.display().to_string(),
        loc,
        functions,
        unwrap_call_sites,
    }))
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
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

fn main() -> Result<()> {
    let scan_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../crates"));

    let mut files = Vec::new();
    collect_rs_files(&scan_root, &mut files)?;

    let mut summaries = Vec::new();
    for f in &files {
        if let Some(s) = analyze_file(f)? {
            summaries.push(s);
        }
    }

    let total_functions: usize = summaries.iter().map(|s| s.functions.len()).sum();
    let total_unwraps: usize = summaries.iter().map(|s| s.unwrap_call_sites.len()).sum();
    let mut most_complex: Vec<(&str, &FunctionSummary)> = summaries
        .iter()
        .flat_map(|s| s.functions.iter().map(move |f| (s.path.as_str(), f)))
        .collect();
    most_complex.sort_by(|a, b| b.1.nesting_depth.cmp(&a.1.nesting_depth));

    eprintln!(
        "扫描 {} 个 .rs 文件，共 {} 个函数，{} 处 .unwrap() 调用",
        summaries.len(),
        total_functions,
        total_unwraps
    );
    eprintln!("嵌套复杂度最高的 10 个函数：");
    for (path, f) in most_complex.iter().take(10) {
        eprintln!(
            "  depth={:<3} {}:{}  {}",
            f.nesting_depth, path, f.start_line, f.name
        );
    }

    println!("{}", serde_json::to_string_pretty(&summaries)?);
    Ok(())
}
