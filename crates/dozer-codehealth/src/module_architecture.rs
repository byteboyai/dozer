//! 从共享 AST 构建 module 图（spec 2026-09-21「module 图」）。
//!
//! 复用 [`crate::report::scan_project`] 已经解析好的 AST 根节点，**不再二次解析**
//! 任何 Rust 文件：
//!
//! - crate root、`foo.rs`、`foo/mod.rs`、`mod foo;` 和内联 `mod foo {}` 建稳定
//!   qualified name 与 module 节点；
//! - 解析 `use crate::`、`self::`、`super::`、workspace crate 前缀、别名和嵌套
//!   use tree；
//! - item 级路径归并到已知的最深 module；解析不到的目标只增加 `unresolved_edges`，
//!   绝不猜测目标；
//! - 同一 from/to 的多条 use 聚合为一条边，证据按路径/行/片段稳定排序去重；
//! - 首版只记录 `cfg` 属性文本，不模拟 feature 组合。
//!
//! 持久化只使用 [`crate::architecture`] 的稳定结构，不泄漏 ast-grep 类型。

use crate::architecture::{
    ArchitectureEdge, ArchitectureEdgeKind, ArchitectureEvidence, ArchitectureNode,
    ArchitectureNodeKind, edge_id, module_node_id,
};
use ast_grep_core::Doc;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 一个 Rust 文件的 module 归属。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileModule {
    /// 文件相对项目根的规范化路径。
    pub path: PathBuf,
}

/// 一次 module 图提取的结果（crate root + module 节点/边）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleGraph {
    pub nodes: Vec<ArchitectureNode>,
    pub edges: Vec<ArchitectureEdge>,
    /// 无法可靠解析的 use 数量（只计数）。
    pub unresolved_edges: usize,
}

/// workspace crate 的模块来源信息，用于把文件路径映射到 crate/module。
#[derive(Debug, Clone)]
pub struct CrateRoots {
    /// crate package 名（segment 前缀用）。
    pub crate_name: String,
    /// crate 的 src 根目录（相对项目根的规范化路径）。
    pub src_dir: PathBuf,
    /// 已知的 root 文件相对路径：`src/lib.rs`、`src/main.rs` 等。
    pub root_files: Vec<PathBuf>,
}

impl CrateRoots {
    /// 单 crate 便捷构造：`crate_name` + `src_dir`，root 文件按惯例推导。
    pub fn conventional(crate_name: impl Into<String>, src_dir: impl Into<PathBuf>) -> Self {
        let crate_name = crate_name.into();
        let src_dir = src_dir.into();
        let root_files = ["lib.rs", "main.rs"]
            .iter()
            .map(|f| src_dir.join(f))
            .collect();
        Self {
            crate_name,
            src_dir,
            root_files,
        }
    }
}

/// 把相对路径转成 `/` 分隔字符串。
fn rel_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 去掉 `.rs` 扩展名后的相对 src 路径字符串（`a/b.rs` → `a/b`）。
fn strip_rs_ext(rel: &Path) -> String {
    let s = rel_str(rel);
    s.strip_suffix(".rs").unwrap_or(&s).to_string()
}

/// 根据 crate 的 `src` 目录和文件相对路径，推出该文件对应的 module 段路径。
///
/// 规则（spec「module 图」）：
/// - `<src>/lib.rs` 或 `<src>/main.rs` → `[]`（crate root）；
/// - `<src>/foo.rs` → `["foo"]`；
/// - `<src>/foo/mod.rs` → `["foo"]`；
/// - `<src>/foo/bar.rs` → `["foo","bar"]`；
/// - 其它嵌套按目录段保留。
pub fn module_segments_for_file(src_dir: &Path, rel: &Path) -> Option<Vec<String>> {
    let rel_from_src = rel.strip_prefix(src_dir).ok()?;
    let no_ext = strip_rs_ext(rel_from_src);
    let segs: Vec<&str> = no_ext.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [] => Some(Vec::new()),
        ["lib"] | ["main"] => Some(Vec::new()),
        rest => {
            let mut out: Vec<String> = rest.iter().map(|s| s.to_string()).collect();
            if out.len() >= 2 && out.last().map(String::as_str) == Some("mod") {
                out.pop();
            }
            Some(out)
        }
    }
}

/// 把 `crate_name` 与模块段拼成限定名（`crate::a::b`；空段 = crate 名本身）。
pub fn qualified_module_name(crate_name: &str, segments: &[String]) -> String {
    if segments.is_empty() {
        crate_name.to_string()
    } else {
        format!("{crate_name}::{}", segments.join("::"))
    }
}

/// 一个已解析的 use 路径：分段（如 `["crate","a","b"]`）与来源信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsePath {
    pub segments: Vec<String>,
    /// `true` = 以 `::` 开头（全局路径）。
    pub global: bool,
    /// 是否来自 `pub use`。
    pub is_pub: bool,
    /// 可选 `cfg` 属性文本。
    pub cfg: Option<String>,
    /// use 声明所在文件的 1-based 行号。
    pub line: usize,
    /// use 声明的短片段（限长）。
    pub snippet: String,
}

/// 构造 use 的规范片段（统一格式，保证证据稳定）。
fn use_snippet(global: bool, is_pub: bool, segments: &[String]) -> String {
    let lead = if global { "::" } else { "" };
    let pub_ = if is_pub { "pub " } else { "" };
    let mut s = format!("{pub_}use {lead}{}", segments.join("::"));
    if s.len() > 120 {
        s.truncate(117);
        s.push_str("...");
    }
    s
}

/// 从 `use` 声明节点递归展开全部叶子路径。
fn expand_use_tree<D: Doc>(
    node: &ast_grep_core::Node<'_, D>,
    prefix: &mut Vec<String>,
    global: bool,
    is_pub: bool,
    cfg: Option<&str>,
    line: usize,
    out: &mut Vec<UsePath>,
) {
    match node.kind().as_ref() {
        "scoped_use_list" => {
            if let Some(path) = node.field("path") {
                push_path_segments(&path, prefix);
            }
            if let Some(list) = node.field("list") {
                for child in list.children() {
                    expand_use_tree(&child, prefix, global, is_pub, cfg, line, out);
                }
            }
        }
        "use_list" => {
            for child in node.children() {
                expand_use_tree(&child, prefix, global, is_pub, cfg, line, out);
            }
        }
        "use_as_clause" => {
            if let Some(path) = node.field("path") {
                expand_use_tree(&path, prefix, global, is_pub, cfg, line, out);
            }
        }
        "use_wildcard" => {
            let mut segs = prefix.clone();
            push_path_segments(node, &mut segs);
            // 通配符本身不是路径段。
            while segs.last().map(String::as_str) == Some("*") {
                segs.pop();
            }
            if !segs.is_empty() {
                out.push(make_use_path(segs, global, is_pub, cfg, line));
            }
        }
        "scoped_identifier" => {
            let mut segs = prefix.clone();
            push_path_segments(node, &mut segs);
            if !segs.is_empty() {
                out.push(make_use_path(segs, global, is_pub, cfg, line));
            }
        }
        "identifier" | "crate" | "self" | "super" | "metavariable" => {
            let mut segs = prefix.clone();
            segs.push(node.text().to_string());
            out.push(make_use_path(segs, global, is_pub, cfg, line));
        }
        _ => {}
    }
}

fn make_use_path(
    segments: Vec<String>,
    global: bool,
    is_pub: bool,
    cfg: Option<&str>,
    line: usize,
) -> UsePath {
    let snippet = use_snippet(global, is_pub, &segments);
    UsePath {
        segments,
        global,
        is_pub,
        cfg: cfg.map(str::to_string),
        line,
        snippet,
    }
}

/// 把路径节点（可能嵌套的 `scoped_identifier`/`use_wildcard`）拆成段并追加到
/// `prefix`。`::` 分隔符与 `*` 通配符本身不算段。
fn push_path_segments<D: Doc>(node: &ast_grep_core::Node<'_, D>, prefix: &mut Vec<String>) {
    match node.kind().as_ref() {
        "scoped_identifier" => {
            if let Some(path) = node.field("path") {
                push_path_segments(&path, prefix);
            }
            if let Some(name) = node.field("name") {
                let t = name.text();
                if t != "::" && t != "*" {
                    prefix.push(t.to_string());
                }
            }
        }
        "use_wildcard" => {
            for child in node.children() {
                push_path_segments(&child, prefix);
            }
        }
        _ => {
            let t = node.text();
            let t = t.strip_prefix("::").unwrap_or(&t);
            if t != "::" && t != "*" && !t.is_empty() {
                prefix.push(t.to_string());
            }
        }
    }
}

/// 提取一个文件里全部 `use` 声明（含 `pub use`、`cfg` 文本、行号）。
pub fn uses_in_file<D: Doc>(root: &ast_grep_core::Node<'_, D>) -> Vec<UsePath> {
    let mut out = Vec::new();
    for decl in root.dfs().filter(|n| n.kind() == "use_declaration") {
        let is_pub = decl
            .children()
            .any(|c| c.kind() == "visibility_modifier" && c.text().starts_with("pub"));
        let cfg = cfg_attribute_of(&decl);
        let line = decl.start_pos().line() + 1;
        let Some(arg) = decl.field("argument") else {
            continue;
        };
        // 全局路径：argument 以 `::` 开头。
        let global = arg.text().trim_start().starts_with("::");
        let mut prefix = Vec::new();
        expand_use_tree(
            &arg,
            &mut prefix,
            global,
            is_pub,
            cfg.as_deref(),
            line,
            &mut out,
        );
    }
    out
}

/// 读取紧邻的 `#[cfg(...)]` 属性文本（不模拟 feature 组合）。
fn cfg_attribute_of<D: Doc>(node: &ast_grep_core::Node<'_, D>) -> Option<String> {
    node.prev()
        .filter(|p| p.kind() == "attribute_item")
        .map(|p| p.text().to_string())
}

/// 一个正在构建的 module 索引：(crate, segments) → 节点 ID。
#[derive(Debug, Default)]
pub struct ModuleIndex {
    by_segments: BTreeMap<(String, Vec<String>), String>,
}

impl ModuleIndex {
    fn insert(&mut self, crate_name: &str, segments: Vec<String>, id: String) {
        self.by_segments
            .insert((crate_name.to_string(), segments), id);
    }

    /// 按 (crate, segments) 查节点 ID；只有精确匹配才算命中。
    fn get_segments(&self, crate_name: &str, segments: &[String]) -> Option<&String> {
        self.by_segments
            .get(&(crate_name.to_string(), segments.to_vec()))
    }
}

/// 构建 module 图。`files` 是每个已分析 Rust 文件的 AST 提取结果；`crate_roots`
/// 提供 workspace crate 的 src/root 信息。调用方保证这些 use 来自与结构/UI 指标
/// 同一次解析的 AST。
pub fn build_module_graph(
    files: &[(FileModule, Vec<UsePath>)],
    crate_roots: &[CrateRoots],
    workspace_crates: &BTreeSet<String>,
) -> ModuleGraph {
    let mut index = ModuleIndex::default();
    let mut nodes: BTreeMap<String, ArchitectureNode> = BTreeMap::new();

    // crate root 节点先按 (crate, []) 注册，ID 复用 `module:<crate>`。
    for roots in crate_roots {
        let id = module_node_id(&roots.crate_name, "");
        index.insert(&roots.crate_name, Vec::new(), id.clone());
        nodes.entry(id.clone()).or_insert_with(|| ArchitectureNode {
            id,
            kind: ArchitectureNodeKind::Crate,
            name: roots.crate_name.clone(),
            qualified_name: roots.crate_name.clone(),
            path: roots.root_files.first().cloned(),
            parent_id: None,
            loc: 0,
            fan_in: 0,
            fan_out: 0,
            layer: None,
            external: false,
        });
    }

    // 先建立所有文件的 (crate, segments) 映射，保证父节点/最深匹配可用。
    let mut file_modules: BTreeMap<PathBuf, (String, Vec<String>)> = BTreeMap::new();
    for (fm, _) in files {
        if let Some((crate_name, segments)) = locate_file(&fm.path, crate_roots) {
            file_modules.insert(fm.path.clone(), (crate_name, segments));
        }
    }
    // 把所有 module 节点插入索引（按 segments 长度升序，父先于子）。
    let mut placements: Vec<(String, Vec<String>)> = file_modules.values().cloned().collect();
    placements.sort_by_key(|(_, segs)| segs.len());
    for (crate_name, segments) in &placements {
        let id = module_node_id(crate_name, &segments.join("::"));
        index.insert(crate_name, segments.clone(), id);
    }
    // 再生成节点（父 ID 依赖索引已就绪）。
    for (path, (crate_name, segments)) in &file_modules {
        let id = module_node_id(crate_name, &segments.join("::"));
        if nodes.contains_key(&id) {
            continue;
        }
        let parent_id = parent_module_id(&index, crate_name, segments);
        let qualified = qualified_module_name(crate_name, segments);
        nodes.entry(id.clone()).or_insert_with(|| ArchitectureNode {
            id,
            kind: ArchitectureNodeKind::Module,
            name: segments
                .last()
                .cloned()
                .unwrap_or_else(|| crate_name.clone()),
            qualified_name: qualified,
            path: Some(path.clone()),
            parent_id,
            loc: 0,
            fan_in: 0,
            fan_out: 0,
            layer: None,
            external: false,
        });
    }

    // 解析 use 边。
    let mut edge_evidence: BTreeMap<(String, String), Vec<ArchitectureEvidence>> = BTreeMap::new();
    let mut unresolved = 0usize;
    for (fm, uses) in files {
        let Some((crate_name, segments)) = file_modules.get(&fm.path) else {
            continue;
        };
        let from_id = module_node_id(crate_name, &segments.join("::"));
        for use_path in uses {
            match resolve_use(crate_name, segments, use_path, &index, workspace_crates) {
                Some(to_id) if to_id != from_id => {
                    edge_evidence
                        .entry((from_id.clone(), to_id))
                        .or_default()
                        .push(ArchitectureEvidence {
                            path: fm.path.clone(),
                            line: use_path.line,
                            snippet: use_path.snippet.clone(),
                        });
                }
                Some(_) => {} // 自依赖：不建边，也不计 unresolved
                None => unresolved += 1,
            }
        }
    }

    let edges = edge_evidence
        .into_iter()
        .map(|((from, to), mut evidence)| {
            evidence.sort();
            evidence.dedup();
            ArchitectureEdge {
                id: edge_id(ArchitectureEdgeKind::ModuleUse, &from, &to),
                from,
                to,
                kind: ArchitectureEdgeKind::ModuleUse,
                evidence,
            }
        })
        .collect();

    ModuleGraph {
        nodes: nodes.into_values().collect(),
        edges,
        unresolved_edges: unresolved,
    }
}

/// 父 module 节点 ID：`crate::a::b` 的父是 `crate::a`，crate root 无父。
fn parent_module_id(index: &ModuleIndex, crate_name: &str, segments: &[String]) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    index
        .get_segments(crate_name, &segments[..segments.len() - 1])
        .cloned()
}

/// 定位一个文件属于哪个 workspace crate、以及它的 module 段。
fn locate_file(path: &Path, roots: &[CrateRoots]) -> Option<(String, Vec<String>)> {
    for r in roots {
        if r.root_files.iter().any(|rf| rf == path) {
            return Some((r.crate_name.clone(), Vec::new()));
        }
        if let Some(segs) = module_segments_for_file(&r.src_dir, path)
            && !segs.is_empty()
        {
            return Some((r.crate_name.clone(), segs));
        }
    }
    None
}

/// 解析一条 use 到目标 module 节点 ID；解析不到返回 `None`（只计数，不猜）。
///
/// 外部 crate 前缀（非 workspace crate）返回 `None`：module 图只连内部节点，
/// 第三方依赖由 Cargo 图表达。
fn resolve_use(
    crate_name: &str,
    from_segments: &[String],
    use_path: &UsePath,
    index: &ModuleIndex,
    workspace_crates: &BTreeSet<String>,
) -> Option<String> {
    let mut segs = use_path.segments.clone();
    if segs.is_empty() {
        return None;
    }
    // 路径起点：
    // - `crate::` / 已知 workspace crate / `self::` / `super::` 的起点一定是已知
    //   module，只需把剩余段归并；
    // - 全局路径指向外部 crate 的，直接不可解析。
    let (target_crate, base_segs) = if use_path.global {
        let first = segs.remove(0);
        if first == "crate" {
            (crate_name.to_string(), Vec::new())
        } else if workspace_crates.contains(&first) {
            (first, Vec::new())
        } else {
            return None;
        }
    } else {
        let first = segs[0].clone();
        match first.as_str() {
            "crate" => {
                segs.remove(0);
                (crate_name.to_string(), Vec::new())
            }
            "self" => {
                segs.remove(0);
                (crate_name.to_string(), from_segments.to_vec())
            }
            "super" => {
                let mut base = from_segments.to_vec();
                while segs.first().map(String::as_str) == Some("super") {
                    segs.remove(0);
                    base.pop();
                }
                (crate_name.to_string(), base)
            }
            _ if workspace_crates.contains(&first) => {
                segs.remove(0);
                (first, Vec::new())
            }
            // 相对路径：从当前 module 出发。
            _ => (crate_name.to_string(), from_segments.to_vec()),
        }
    };

    // 把 use 的剩余段追加到起始 module 段，精确匹配已知 module；若整段不中，
    // 去掉最后一段（item 名）再试一次。不会退回到 crate root，避免把
    // `crate::nope::Missing` 之类误判为已解析。
    let mut full = base_segs;
    let base_len = full.len();
    full.extend(segs.iter().cloned());
    if let Some(id) = index.get_segments(&target_crate, &full) {
        return Some(id.clone());
    }
    if full.len() > base_len {
        full.pop();
        if let Some(id) = index.get_segments(&target_crate, &full) {
            return Some(id.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_language::{LanguageExt, SupportLang};

    fn uses_of(src: &str) -> Vec<UsePath> {
        let ast = SupportLang::Rust.ast_grep(src);
        uses_in_file(&ast.root())
    }

    fn segs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn crate_root_files_map_to_empty_segments() {
        let src = PathBuf::from("crates/a/src");
        assert_eq!(
            module_segments_for_file(&src, Path::new("crates/a/src/lib.rs")),
            Some(Vec::new())
        );
        assert_eq!(
            module_segments_for_file(&src, Path::new("crates/a/src/main.rs")),
            Some(Vec::new())
        );
    }

    #[test]
    fn nested_file_paths_map_to_module_segments() {
        let src = PathBuf::from("crates/a/src");
        assert_eq!(
            module_segments_for_file(&src, Path::new("crates/a/src/foo.rs")),
            Some(segs(&["foo"]))
        );
        assert_eq!(
            module_segments_for_file(&src, Path::new("crates/a/src/foo/mod.rs")),
            Some(segs(&["foo"]))
        );
        assert_eq!(
            module_segments_for_file(&src, Path::new("crates/a/src/foo/bar.rs")),
            Some(segs(&["foo", "bar"]))
        );
    }

    #[test]
    fn qualified_name_joins_segments() {
        assert_eq!(qualified_module_name("a", &[]), "a");
        assert_eq!(
            qualified_module_name("a", &segs(&["foo", "bar"])),
            "a::foo::bar"
        );
    }

    #[test]
    fn parses_simple_crate_use() {
        let uses = uses_of("use crate::foo::bar;\n");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].segments, segs(&["crate", "foo", "bar"]));
        assert!(!uses[0].global);
        assert_eq!(uses[0].line, 1);
    }

    #[test]
    fn parses_self_super_and_global() {
        assert_eq!(uses_of("use self::x;\n")[0].segments, segs(&["self", "x"]));
        assert_eq!(
            uses_of("use super::x;\n")[0].segments,
            segs(&["super", "x"])
        );
        let g = uses_of("use ::serde::json;\n");
        assert_eq!(g[0].segments, segs(&["serde", "json"]));
        assert!(g[0].global);
    }

    #[test]
    fn parses_grouped_use_into_leaves() {
        let uses = uses_of("use crate::a::{b, c::d};\n");
        let mut paths: Vec<Vec<String>> = uses.iter().map(|u| u.segments.clone()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![segs(&["crate", "a", "b"]), segs(&["crate", "a", "c", "d"])]
        );
    }

    #[test]
    fn parses_alias_and_glob_and_pub_use() {
        let aliased = uses_of("use crate::a::b as c;\n");
        assert_eq!(aliased[0].segments, segs(&["crate", "a", "b"]));
        let glob = uses_of("use crate::a::*;\n");
        assert_eq!(glob[0].segments, segs(&["crate", "a"]));
        let pub_use = uses_of("pub use crate::a::b;\n");
        assert!(pub_use[0].is_pub);
        assert!(pub_use[0].snippet.starts_with("pub use"));
    }

    #[test]
    fn captures_cfg_attribute_text() {
        let uses = uses_of("#[cfg(feature = \"x\")]\nuse crate::a::b;\n");
        assert_eq!(uses[0].cfg.as_deref(), Some("#[cfg(feature = \"x\")]"));
    }

    #[test]
    fn cfg_attribute_not_captured_when_far_away() {
        let uses = uses_of("use crate::a::b;\n#[cfg(unix)]\nfn f() {}\n");
        assert!(uses[0].cfg.is_none());
    }

    #[test]
    fn root_use_tree_with_nested_group() {
        let uses = uses_of("use crate::{a, b::{c, d}};\n");
        let mut paths: Vec<Vec<String>> = uses.iter().map(|u| u.segments.clone()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                segs(&["crate", "a"]),
                segs(&["crate", "b", "c"]),
                segs(&["crate", "b", "d"])
            ]
        );
    }

    fn roots_a() -> Vec<CrateRoots> {
        vec![CrateRoots::conventional("a", "crates/a/src")]
    }

    fn one_crate() -> BTreeSet<String> {
        ["a".to_string()].into_iter().collect()
    }

    #[test]
    fn builds_crate_root_and_module_nodes() {
        let files = vec![
            (
                FileModule {
                    path: "crates/a/src/lib.rs".into(),
                },
                vec![],
            ),
            (
                FileModule {
                    path: "crates/a/src/foo.rs".into(),
                },
                vec![],
            ),
        ];
        let g = build_module_graph(&files, &roots_a(), &one_crate());
        assert!(g.nodes.iter().any(|n| n.id == module_node_id("a", "")));
        let foo = g
            .nodes
            .iter()
            .find(|n| n.id == module_node_id("a", "foo"))
            .expect("foo module node");
        assert_eq!(foo.qualified_name, "a::foo");
        assert_eq!(
            foo.parent_id.as_deref(),
            Some(module_node_id("a", "").as_str())
        );
    }

    #[test]
    fn resolves_crate_use_to_deepest_module() {
        let files = vec![
            (
                FileModule {
                    path: "crates/a/src/lib.rs".into(),
                },
                uses_of("use crate::foo::Bar;\n"),
            ),
            (
                FileModule {
                    path: "crates/a/src/foo.rs".into(),
                },
                vec![],
            ),
        ];
        let g = build_module_graph(&files, &roots_a(), &one_crate());
        let edge = g
            .edges
            .iter()
            .find(|e| e.from == module_node_id("a", "") && e.to == module_node_id("a", "foo"));
        let edge = edge.expect("crate -> foo edge");
        assert_eq!(edge.kind, ArchitectureEdgeKind::ModuleUse);
        assert_eq!(edge.evidence[0].line, 1);
        assert!(edge.evidence[0].snippet.contains("crate::foo::Bar"));
        assert_eq!(g.unresolved_edges, 0);
    }

    #[test]
    fn aggregates_multiple_uses_same_pair() {
        let files = vec![
            (
                FileModule {
                    path: "crates/a/src/lib.rs".into(),
                },
                uses_of("use crate::foo::A;\nuse crate::foo::B;\n"),
            ),
            (
                FileModule {
                    path: "crates/a/src/foo.rs".into(),
                },
                vec![],
            ),
        ];
        let g = build_module_graph(&files, &roots_a(), &one_crate());
        let count = g
            .edges
            .iter()
            .filter(|e| e.from == module_node_id("a", "") && e.to == module_node_id("a", "foo"))
            .count();
        assert_eq!(count, 1);
        let edge = g
            .edges
            .iter()
            .find(|e| e.from == module_node_id("a", "") && e.to == module_node_id("a", "foo"))
            .unwrap();
        assert_eq!(edge.evidence.len(), 2);
    }

    #[test]
    fn unresolved_use_only_counted_not_guessed() {
        let files = vec![(
            FileModule {
                path: "crates/a/src/lib.rs".into(),
            },
            uses_of("use serde::Serialize;\nuse crate::nope::Missing;\n"),
        )];
        let g = build_module_graph(&files, &roots_a(), &one_crate());
        // serde 是 external、nope 不存在，都不产生边。
        assert!(g.edges.is_empty());
        assert_eq!(g.unresolved_edges, 2);
    }

    #[test]
    fn cross_workspace_crate_use_resolves_to_crate_root() {
        let roots = vec![
            CrateRoots::conventional("a", "crates/a/src"),
            CrateRoots::conventional("b", "crates/b/src"),
        ];
        let crates: BTreeSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
        let files = vec![(
            FileModule {
                path: "crates/a/src/lib.rs".into(),
            },
            uses_of("use b::Thing;\n"),
        )];
        let g = build_module_graph(&files, &roots, &crates);
        let edge = g
            .edges
            .iter()
            .find(|e| e.from == module_node_id("a", "") && e.to == module_node_id("b", ""))
            .expect("a -> b edge");
        assert_eq!(edge.kind, ArchitectureEdgeKind::ModuleUse);
        assert_eq!(g.unresolved_edges, 0);
    }

    #[test]
    fn super_from_nested_module_resolves_to_parent() {
        let files = vec![
            (
                FileModule {
                    path: "crates/a/src/lib.rs".into(),
                },
                vec![],
            ),
            (
                FileModule {
                    path: "crates/a/src/foo.rs".into(),
                },
                vec![],
            ),
            (
                FileModule {
                    path: "crates/a/src/foo/bar.rs".into(),
                },
                uses_of("use super::Thing;\n"),
            ),
        ];
        let g = build_module_graph(&files, &roots_a(), &one_crate());
        let edge = g
            .edges
            .iter()
            .find(|e| e.from == module_node_id("a", "foo::bar"))
            .expect("bar module edge");
        assert_eq!(edge.to, module_node_id("a", "foo"));
    }
}
