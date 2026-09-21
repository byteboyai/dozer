//! Cargo workspace 架构提取（spec 2026-09-21「Cargo workspace 图」）。
//!
//! 以项目根目录的 `Cargo.toml` 为入口，用 `cargo_metadata` 读取 workspace：
//! 只把 workspace member 建为内部 crate 节点，member 之间的依赖建
//! `CargoDependency` 边；直接第三方依赖可建 external 节点（`external = true`，
//! 默认不在 UI 展示）。
//!
//! - 用 `--no-deps` 等价能力（[`cargo_metadata::MetadataCommand::no_deps`]），
//!   避免下载依赖或扩大扫描范围。
//! - target-specific / optional / renamed 依赖保留在证据属性中，首版 UI 聚合展示。
//! - metadata 命令失败时返回 [`CargoArchitecture::Failed`]，让调用方把架构状态
//!   置 `Partial` 但保留后续 module 图，而不是让整次扫描失败。
//! - `dozer-codehealth` 是纯分析库：这里只读 metadata，不做任何写操作。

use crate::architecture::{
    ArchitectureEdge, ArchitectureEdgeKind, ArchitectureEvidence, ArchitectureNode,
    ArchitectureNodeKind, ArchitectureStatus, crate_node_id, edge_id, external_node_id,
};
use cargo_metadata::{Metadata, MetadataCommand, Package};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Cargo 架构提取结果。失败可降级，不阻断 module 图。
#[derive(Debug, Clone, PartialEq)]
pub enum CargoArchitecture {
    /// 未发现 Cargo workspace（非 Cargo 项目，或缺 manifest）。
    NotApplicable,
    /// 成功读取 workspace。
    Ok(CargoGraph),
    /// metadata 命令失败；携带可展示的错误文案。
    Failed(String),
}

/// 从 workspace 提取出的 crate 图。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CargoGraph {
    pub nodes: Vec<ArchitectureNode>,
    pub edges: Vec<ArchitectureEdge>,
    /// workspace 根目录（用于把 manifest 路径转成相对项目根的路径）。
    pub workspace_root: PathBuf,
    /// 项目根内的 workspace member 数量（不含 external）。
    pub member_count: usize,
}

/// 项目根里用于查找 workspace 的候选 manifest 相对路径。
const CANDIDATE_MANIFESTS: &[&str] = &["Cargo.toml"];

/// 在 `root` 下查找 Cargo manifest。只认根目录的 `Cargo.toml`（首版不递归猜
/// 子目录里的独立 workspace，避免把 vendored crate 当成项目架构）。
pub fn find_manifest(root: &Path) -> Option<PathBuf> {
    CANDIDATE_MANIFESTS
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.is_file())
}

/// 读取 workspace 架构。缺 manifest → `NotApplicable`；命令失败 → `Failed`。
pub fn extract_cargo_architecture(root: &Path) -> CargoArchitecture {
    let Some(manifest) = find_manifest(root) else {
        return CargoArchitecture::NotApplicable;
    };
    match run_metadata(&manifest) {
        Ok(metadata) => CargoArchitecture::Ok(graph_from_metadata(root, &metadata)),
        Err(e) => CargoArchitecture::Failed(e),
    }
}

/// 调用 `cargo metadata --no-deps`。`--no-deps` 避免下载依赖。
fn run_metadata(manifest: &Path) -> Result<Metadata, String> {
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest);
    cmd.no_deps();
    cmd.exec().map_err(|e| format!("cargo metadata 失败：{e}"))
}

/// 规范化路径：相对项目根，`/` 分隔；不在根内则原样返回。
fn rel_path(root: &Path, path: &Path) -> PathBuf {
    let rel = path.strip_prefix(root).unwrap_or(path);
    PathBuf::from(rel.to_string_lossy().replace('\\', "/"))
}

/// 把 manifest 路径所在目录变成可读的短片段（`crates/foo/Cargo.toml`）。
fn manifest_snippet(root: &Path, manifest: &Path) -> String {
    rel_path(root, manifest).to_string_lossy().into_owned()
}

/// 一个 workspace member 的 package name → 节点 ID（同名时附加 manifest 相对
/// 路径消歧，spec「稳定标识」）。
fn member_node_id(root: &Path, pkg: &Package, name_collision: bool) -> String {
    let base = crate_node_id(&pkg.name);
    if name_collision {
        let rel = rel_path(root, pkg.manifest_path.as_std_path());
        format!("{base}#{}", rel.to_string_lossy())
    } else {
        base
    }
}

/// 从 `cargo_metadata` 结果构建 crate 图。
pub fn graph_from_metadata(root: &Path, metadata: &Metadata) -> CargoGraph {
    let workspace_root = metadata.workspace_root.as_std_path().to_path_buf();

    // workspace member 集合：用 package id 判定，避免把非 member 的 path 依赖
    // （例如 `spike/*` 之外被 exclude 的 crate）误建为内部节点。
    let member_ids: std::collections::HashSet<&str> = metadata
        .workspace_members
        .iter()
        .map(|id| id.repr.as_str())
        .collect();

    // 检测 package name 冲突（同名 workspace member）以便 ID 消歧。
    let mut name_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for pkg in &metadata.packages {
        if member_ids.contains(pkg.id.repr.as_str()) {
            *name_counts.entry(pkg.name.as_str()).or_insert(0) += 1;
        }
    }

    // 内部 member：name → 节点 ID（每个 workspace package 名只会出现一次，
    // 除非同名，此时消歧后缀也只用于同名的那几个）。
    let mut member_name_to_id: BTreeMap<String, String> = BTreeMap::new();
    let mut nodes: Vec<ArchitectureNode> = Vec::new();
    let mut member_packages: Vec<&Package> = metadata
        .packages
        .iter()
        .filter(|p| member_ids.contains(p.id.repr.as_str()))
        .collect();
    member_packages.sort_by(|a, b| a.name.cmp(&b.name));

    for pkg in &member_packages {
        let collision = name_counts.get(pkg.name.as_str()).copied().unwrap_or(0) > 1;
        let id = member_node_id(root, pkg, collision);
        // 同名 member 用 manifest 相对路径作 qualified_name 区分。
        let qualified_name = if collision {
            format!(
                "{} ({})",
                pkg.name,
                manifest_snippet(root, pkg.manifest_path.as_std_path())
            )
        } else {
            pkg.name.clone()
        };
        member_name_to_id.insert(pkg.name.clone(), id.clone());
        nodes.push(ArchitectureNode {
            id,
            kind: ArchitectureNodeKind::Crate,
            name: pkg.name.clone(),
            qualified_name,
            path: Some(rel_path(root, pkg.manifest_path.as_std_path())),
            parent_id: None,
            loc: 0,
            fan_in: 0,
            fan_out: 0,
            layer: None,
            external: false,
        });
    }

    let member_count = member_packages.len();

    // 边聚合：(kind, from, to) → 证据。
    let mut edge_evidence: BTreeMap<
        (ArchitectureEdgeKind, String, String),
        Vec<ArchitectureEvidence>,
    > = BTreeMap::new();
    // external 节点（直接第三方依赖），name → id。
    let mut external_nodes: BTreeMap<String, ArchitectureNode> = BTreeMap::new();

    for pkg in &member_packages {
        let Some(from_id) = member_name_to_id.get(&pkg.name) else {
            continue;
        };
        let manifest_rel = rel_path(root, pkg.manifest_path.as_std_path());
        for dep in &pkg.dependencies {
            // rename 时 manifest 里写的是 rename 名，真实 package name 在 `name`。
            let dep_package = dep.name.as_str();
            let snippet = dependency_snippet(dep);

            if let Some(to_id) = member_name_to_id.get(dep_package) {
                // 内部 member 依赖 → CargoDependency 边。
                if to_id == from_id {
                    continue; // 自依赖不建边（Cargo 也不允许）
                }
                edge_evidence
                    .entry((
                        ArchitectureEdgeKind::CargoDependency,
                        from_id.clone(),
                        to_id.clone(),
                    ))
                    .or_default()
                    .push(ArchitectureEvidence {
                        path: manifest_rel.clone(),
                        line: 1,
                        snippet: snippet.clone(),
                    });
            } else {
                // 直接第三方依赖 → external 节点（默认 UI 不展示）。
                let ext_id = external_node_id(dep_package);
                external_nodes
                    .entry(dep_package.to_string())
                    .or_insert_with(|| ArchitectureNode {
                        id: ext_id.clone(),
                        kind: ArchitectureNodeKind::ExternalCrate,
                        name: dep_package.to_string(),
                        qualified_name: dep_package.to_string(),
                        path: None,
                        parent_id: None,
                        loc: 0,
                        fan_in: 0,
                        fan_out: 0,
                        layer: None,
                        external: true,
                    });
                edge_evidence
                    .entry((
                        ArchitectureEdgeKind::CargoDependency,
                        from_id.clone(),
                        ext_id,
                    ))
                    .or_default()
                    .push(ArchitectureEvidence {
                        path: manifest_rel.clone(),
                        line: 1,
                        snippet,
                    });
            }
        }
    }

    nodes.extend(external_nodes.into_values());

    let edges = edge_evidence
        .into_iter()
        .map(|((kind, from, to), mut evidence)| {
            evidence.sort();
            evidence.dedup();
            ArchitectureEdge {
                id: edge_id(kind, &from, &to),
                from,
                to,
                kind,
                evidence,
            }
        })
        .collect();

    CargoGraph {
        nodes,
        edges,
        workspace_root,
        member_count,
    }
}

/// 依赖声明的短片段：`name`、rename、optional、target-specific 都保留在文本里，
/// 供详情面板回溯（首版不模拟 feature/target 解析）。
fn dependency_snippet(dep: &cargo_metadata::Dependency) -> String {
    let mut s = match &dep.rename {
        Some(rename) => format!("{rename} = {name}", name = dep.name),
        None => dep.name.clone(),
    };
    if dep.optional {
        s.push_str(" (optional)");
    }
    if let Some(target) = &dep.target {
        s.push_str(&format!(" [{target}]"));
    }
    s
}

/// 把 Cargo 提取结果转成架构报告的初始片段与状态。
///
/// - `Ok`：节点/边/状态 `Complete`（module 图会在后续任务里合并）。
/// - `NotApplicable`：无 Cargo workspace；状态 `NotApplicable`（非 Rust 项目
///   仍可能由 module 图升级为 `Partial`/`Complete`）。
/// - `Failed`：状态 `Partial`，错误文案由调用方记入扫描范围。
pub fn cargo_report_fragment(
    cargo: &CargoArchitecture,
) -> (
    ArchitectureStatus,
    Vec<ArchitectureNode>,
    Vec<ArchitectureEdge>,
    Option<String>,
) {
    match cargo {
        CargoArchitecture::NotApplicable => (
            ArchitectureStatus::NotApplicable,
            Vec::new(),
            Vec::new(),
            None,
        ),
        CargoArchitecture::Failed(err) => (
            ArchitectureStatus::Partial,
            Vec::new(),
            Vec::new(),
            Some(err.clone()),
        ),
        CargoArchitecture::Ok(graph) => (
            ArchitectureStatus::Complete,
            graph.nodes.clone(),
            graph.edges.clone(),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    /// 建一个临时 workspace：`a`（无依赖）、`b`（依赖 a），外加第三方依赖 `serde`。
    fn two_member_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        for (name, extra) in [("a", ""), ("b", "a = { path = \"../a\" }\nserde = \"1\"\n")] {
            let pkg = root.join(name);
            fs::create_dir_all(pkg.join("src")).unwrap();
            fs::write(
                pkg.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{extra}"),
            )
            .unwrap();
            fs::write(pkg.join("src/lib.rs"), "").unwrap();
        }
        dir
    }

    #[test]
    fn missing_manifest_is_not_applicable() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            extract_cargo_architecture(dir.path()),
            CargoArchitecture::NotApplicable
        );
    }

    #[test]
    fn workspace_members_become_crate_nodes() {
        let dir = two_member_workspace();
        let CargoArchitecture::Ok(graph) = extract_cargo_architecture(dir.path()) else {
            panic!("expected Cargo graph");
        };
        assert_eq!(graph.member_count, 2);
        let internal: Vec<_> = graph.nodes.iter().filter(|n| !n.external).collect();
        assert_eq!(internal.len(), 2);
        assert!(internal.iter().any(|n| n.id == crate_node_id("a")));
        assert!(internal.iter().any(|n| n.id == crate_node_id("b")));
        assert!(
            internal
                .iter()
                .all(|n| n.kind == ArchitectureNodeKind::Crate)
        );
    }

    #[test]
    fn member_dependency_becomes_cargo_edge_with_evidence() {
        let dir = two_member_workspace();
        let CargoArchitecture::Ok(graph) = extract_cargo_architecture(dir.path()) else {
            panic!("expected Cargo graph");
        };
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from == crate_node_id("b") && e.to == crate_node_id("a"))
            .expect("b -> a edge");
        assert_eq!(edge.kind, ArchitectureEdgeKind::CargoDependency);
        assert_eq!(
            edge.id,
            edge_id(ArchitectureEdgeKind::CargoDependency, "crate:b", "crate:a")
        );
        assert!(!edge.evidence.is_empty());
        assert!(
            edge.evidence
                .iter()
                .all(|e| e.path.ends_with("b/Cargo.toml"))
        );
    }

    #[test]
    fn third_party_dependency_becomes_external_node() {
        let dir = two_member_workspace();
        let CargoArchitecture::Ok(graph) = extract_cargo_architecture(dir.path()) else {
            panic!("expected Cargo graph");
        };
        let serde = graph
            .nodes
            .iter()
            .find(|n| n.id == external_node_id("serde"))
            .expect("serde external node");
        assert!(serde.external);
        assert_eq!(serde.kind, ArchitectureNodeKind::ExternalCrate);
        assert!(
            graph
                .edges
                .iter()
                .any(|e| { e.from == crate_node_id("b") && e.to == external_node_id("serde") })
        );
    }

    #[test]
    fn repeated_crate_edge_aggregates_into_one() {
        let dir = two_member_workspace();
        let CargoArchitecture::Ok(graph) = extract_cargo_architecture(dir.path()) else {
            panic!("expected Cargo graph");
        };
        // b -> a 只应有一条边，即使证据可能来自多行。
        let count = graph
            .edges
            .iter()
            .filter(|e| e.from == crate_node_id("b") && e.to == crate_node_id("a"))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn node_ids_are_unique_across_kinds() {
        let dir = two_member_workspace();
        let CargoArchitecture::Ok(graph) = extract_cargo_architecture(dir.path()) else {
            panic!("expected Cargo graph");
        };
        let mut ids: Vec<_> = graph.nodes.iter().map(|n| n.id.clone()).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "节点 ID 必须唯一");
    }

    #[test]
    fn optional_and_target_specific_are_recorded_in_snippet() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[target.'cfg(unix)'.dependencies]\nlibc = { version = \"0.2\", optional = true }\n",
        )
        .unwrap();
        fs::write(root.join("a/src/lib.rs"), "").unwrap();
        let CargoArchitecture::Ok(graph) = extract_cargo_architecture(root) else {
            panic!("expected Cargo graph");
        };
        let edge = graph
            .edges
            .iter()
            .find(|e| e.to == external_node_id("libc"))
            .expect("libc edge");
        let snippet = &edge.evidence[0].snippet;
        assert!(snippet.contains("libc"));
        assert!(snippet.contains("optional"), "snippet={snippet}");
        assert!(snippet.contains("cfg(unix)"), "snippet={snippet}");
    }
}
