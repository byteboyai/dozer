//! Spike：验证"UI 复杂度检测"设计里三个没验证过的技术风险，能过就把结论写回
//! spec，验证完这个 spike 就可以删（同 `spike/code-analysis` 的既有惯例）。
//!
//! 跑法：`cargo run -p spike-ui-complexity -- <要扫描的目录，默认 ../../crates>`
//!
//! 验证三件事：
//! 1. 能不能用 ast-grep 的 meta-variable 模式（`$X`）区分"字面量参数"和"变量/
//!    表达式参数"——`.padding(8)` 该被抓，`.padding(spacing)` 不该。
//! 2. 能不能做"结构指纹"去重——忽略字面量具体值、只比较节点种类排列，找出
//!    "形状相同的 widget 构造"在项目里重复了几次。
//! 3. 能不能测"widget 宏嵌套深度"——`row!`/`column!` 互相嵌套了几层。
//!
//! 附带验证两个低风险项（复用 #1 的技术）：事件回调调用密度
//! （`.on_press`/`.on_enter`/`.on_exit` 计数）、调色板/间距值种类数（对 #1
//! 抓到的字面量文本去重计数）。
//!
//! **重要发现（写回 spec 的"已知风险"一节）**：`Node::find_all` 接受
//! `impl Matcher`，`&str` 也实现了 `Matcher`——但 `impl Matcher for str` 内部
//! 每次 `match_node` 调用都会重新 `Pattern::new(self, lang)`（见
//! `ast-grep-core::matcher::Matcher for str`），而 `find_all` 对树里*每个
//! 节点*都调用一次 `match_node`。也就是说直接把 `&str` 传给 `find_all` 会
//! 导致"每个节点都重新编译一次 pattern"，在 379 行的单个文件上就要 6+ 秒
//! （首次实测，10 个 pattern × 全部节点）。正确用法是用
//! `ast_grep_core::matcher::Pattern::new(src, lang)` 在文件循环外把每个
//! pattern 预编译一次，循环内传 `&Pattern`（`Pattern` 实现 `Matcher`，且带
//! `potential_kinds()` 快速跳过不可能匹配的节点）。预编译后全仓库
//! （195 个文件、约 10 万行）扫描耗时从"卡死"降到几秒量级。

use ast_grep_core::matcher::Pattern;
use ast_grep_core::{Doc, Node};
use ast_grep_language::{LanguageExt, SupportLang};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
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

const LITERAL_ARG_KINDS: &[&str] = &[
    "integer_literal",
    "float_literal",
    "array_expression",
    "unary_expression", // 负数字面量在 tree-sitter-rust 里是 unary_expression(-N)
];

/// 预编译好的 pattern 集合，文件循环外只建一次（见文件头部"重要发现"）。
struct Patterns {
    padding: Pattern,
    spacing: Pattern,
    color_ctors: [Pattern; 3],
    event_handlers: [Pattern; 5],
}

impl Patterns {
    fn compile(lang: SupportLang) -> Self {
        Patterns {
            padding: Pattern::new("$RECV.padding($X)", lang),
            spacing: Pattern::new("$RECV.spacing($X)", lang),
            color_ctors: [
                Pattern::new("Color::from_rgb($$$)", lang),
                Pattern::new("Color::from_rgba($$$)", lang),
                Pattern::new("Color::from_rgb8($$$)", lang),
            ],
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

/// 验证 #1：`.padding($X)`/`.spacing($X)` 的 `$X` 能不能被拿到、能不能按 kind
/// 区分字面量 vs 变量/表达式引用。
fn find_literal_style_args(root: &Node<'_, impl Doc>, pat: &Pattern) -> Vec<(usize, String)> {
    root.find_all(pat)
        .filter_map(|m| {
            let arg = m.get_env().get_match("X")?;
            if LITERAL_ARG_KINDS.contains(&arg.kind().as_ref()) {
                Some((m.get_node().start_pos().line() + 1, arg.text().to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// 验证 #1（颜色变体）：构造函数调用本身就是"硬编码颜色"信号，不需要像
/// padding/spacing 那样区分参数是不是字面量。
fn find_color_constructor_calls(root: &Node<'_, impl Doc>, pats: &[Pattern; 3]) -> usize {
    pats.iter().map(|p| root.find_all(p).count()).sum()
}

/// 验证 #3：`row!`/`column!` 宏调用的嵌套深度。tree-sitter-rust 里宏调用节点
/// 是 `macro_invocation`，宏名在 `macro` 字段。
fn macro_nesting_depth(node: &Node<'_, impl Doc>, target_macros: &[&str]) -> usize {
    fn walk<D: Doc>(node: &Node<'_, D>, target_macros: &[&str], depth: usize) -> usize {
        let is_target = node.kind() == "macro_invocation"
            && node
                .field("macro")
                .map(|m| target_macros.contains(&m.text().as_ref()))
                .unwrap_or(false);
        let next_depth = if is_target { depth + 1 } else { depth };
        node.children()
            .map(|c| walk(&c, target_macros, next_depth))
            .max()
            .unwrap_or(next_depth)
    }
    walk(node, target_macros, 0)
}

/// 验证 #2：结构指纹——把一个节点子树序列化成"只保留 kind 排列、丢弃字面量
/// 文本"的字符串。两个 widget 构造表达式如果指纹相同，说明"形状"一样（即使
/// 传的文字/数字不同），是"该抽组件"的候选。
fn structural_fingerprint<D: Doc>(node: &Node<'_, D>) -> String {
    let mut out = String::new();
    fn walk<D: Doc>(node: &Node<'_, D>, out: &mut String) {
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

/// 验证 #2 的载体：`row!`/`column!` 宏调用本身（不含更深层嵌套的同类宏，避免
/// 一个大结构和它内部的子结构互相污染彼此的计数——只指纹"顶层"widget 构造）。
fn top_level_widget_macros<'t, D: Doc>(
    node: &Node<'t, D>,
    target_macros: &[&str],
) -> Vec<Node<'t, D>> {
    let mut out = Vec::new();
    fn walk<'t, D: Doc>(node: &Node<'t, D>, target_macros: &[&str], out: &mut Vec<Node<'t, D>>) {
        let is_target = node.kind() == "macro_invocation"
            && node
                .field("macro")
                .map(|m| target_macros.contains(&m.text().as_ref()))
                .unwrap_or(false);
        if is_target {
            out.push(node.clone());
            return; // 顶层命中就不下钻,子节点的同类宏由它自己在别处作为"顶层"被访问到
        }
        for c in node.children() {
            walk(&c, target_macros, out);
        }
    }
    walk(node, target_macros, &mut out);
    out
}

/// 附带验证：事件回调密度，`.on_press`/`.on_enter`/`.on_exit` 等方法调用计数
/// （复用 #1 的 `find_all` 技术，风险已经在 #1 验证过，这里只是换个 pattern）。
fn event_handler_call_count(root: &Node<'_, impl Doc>, pats: &[Pattern; 5]) -> usize {
    pats.iter().map(|p| root.find_all(p).count()).sum()
}

fn main() -> anyhow::Result<()> {
    let scan_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../crates"));

    let mut files = Vec::new();
    collect_rs_files(&scan_root, &mut files)?;

    let patterns = Patterns::compile(SupportLang::Rust);

    let mut total_literal_padding = 0usize;
    let mut total_literal_spacing = 0usize;
    let mut total_color_ctor = 0usize;
    let mut total_event_handlers = 0usize;
    let mut distinct_literal_values: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut fingerprint_counts: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    let mut max_depth_seen: Vec<(String, usize)> = Vec::new(); // (file, depth)

    let start = Instant::now();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let root = SupportLang::Rust.ast_grep(&src);
        let root_node = root.root();

        let padding_hits = find_literal_style_args(&root_node, &patterns.padding);
        let spacing_hits = find_literal_style_args(&root_node, &patterns.spacing);
        total_literal_padding += padding_hits.len();
        total_literal_spacing += spacing_hits.len();
        total_color_ctor += find_color_constructor_calls(&root_node, &patterns.color_ctors);
        for (_, v) in padding_hits.iter().chain(spacing_hits.iter()) {
            distinct_literal_values.insert(v.clone());
        }

        total_event_handlers += event_handler_call_count(&root_node, &patterns.event_handlers);

        let widgets = top_level_widget_macros(&root_node, &["row", "column"]);
        for w in &widgets {
            let fp = structural_fingerprint(w);
            fingerprint_counts
                .entry(fp)
                .or_default()
                .push((path.display().to_string(), w.start_pos().line() + 1));
        }

        let depth = macro_nesting_depth(&root_node, &["row", "column"]);
        if depth > 0 {
            max_depth_seen.push((path.display().to_string(), depth));
        }
    }
    let elapsed = start.elapsed();

    eprintln!(
        "扫描 {} 个 .rs 文件（{}），耗时 {:.2}s",
        files.len(),
        scan_root.display(),
        elapsed.as_secs_f64()
    );

    eprintln!("\n== 验证 #1：字面量参数检测 ==");
    eprintln!("  .padding(字面量) 命中数: {total_literal_padding}");
    eprintln!("  .spacing(字面量) 命中数: {total_literal_spacing}");
    eprintln!("  Color::from_rgb*(...) 构造调用数: {total_color_ctor}");
    eprintln!(
        "  distinct padding/spacing 字面量值种类数: {}",
        distinct_literal_values.len()
    );

    eprintln!("\n== 附带验证：事件回调密度 ==");
    eprintln!("  on_press/on_enter/on_exit/on_input/on_submit 总调用数: {total_event_handlers}");

    eprintln!("\n== 验证 #3：row!/column! 嵌套深度 ==");
    max_depth_seen.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("  嵌套深度最高的 10 个文件：");
    for (path, depth) in max_depth_seen.iter().take(10) {
        eprintln!("    depth={depth:<3} {path}");
    }

    eprintln!("\n== 验证 #2：结构指纹去重（row!/column! 顶层构造）==");
    let mut dup_clusters: Vec<(&String, &Vec<(String, usize)>)> = fingerprint_counts
        .iter()
        .filter(|(_, locs)| locs.len() >= 3)
        .collect();
    dup_clusters.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    eprintln!("  重复次数 >=3 的结构指纹簇数: {}", dup_clusters.len());
    for (fp, locs) in dup_clusters.iter().take(5) {
        eprintln!(
            "    出现 {} 次，指纹长度 {}，前 3 处位置: {:?}",
            locs.len(),
            fp.len(),
            &locs[..3.min(locs.len())]
        );
    }

    Ok(())
}
