//! 架构差异与变更影响范围（spec 2026-09-21「统一发现与差异」「影响范围」）。
//!
//! 与 [`crate::diff`] 的发现差异分开：节点/边/环的增删按稳定 ID 比较，独立于
//! Finding。首次扫描（或旧 schema 无架构字段）时返回显式
//! [`ArchitectureDiffOutcome::NoBaseline`]，UI 不得把所有现存边渲染成“本轮新增”。
//!
//! 影响范围沿**反向**内部依赖边做有深度上限的 BFS：从 Git dirty 文件与相对上次
//! 快照发生变化的 module 出发，区分直接（距离 1）与间接（距离 ≥ 2）受影响节点，
//! 记录最短距离，并限制访问节点总数防止异常大图拖慢 UI。

use crate::architecture::{ArchitectureEdge, ArchitectureReport};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;

/// 架构差异比较结果。所有向量按稳定 ID 排序。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArchitectureDiff {
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
    pub added_cycles: Vec<String>,
    pub resolved_cycles: Vec<String>,
}

impl ArchitectureDiff {
    pub fn is_empty(&self) -> bool {
        self.added_nodes.is_empty()
            && self.removed_nodes.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
            && self.added_cycles.is_empty()
            && self.resolved_cycles.is_empty()
    }
}

/// 差异结果：区分“有基线可比较”与“无基线”（首次扫描 / 旧 schema）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ArchitectureDiffOutcome {
    /// 没有可用的上一份 schema v3 架构报告，无法给出变化。
    #[default]
    NoBaseline,
    /// 与上一份报告比较得到的变化（可能为空）。
    Compared(ArchitectureDiff),
}

impl ArchitectureDiffOutcome {
    pub fn diff(&self) -> Option<&ArchitectureDiff> {
        match self {
            ArchitectureDiffOutcome::NoBaseline => None,
            ArchitectureDiffOutcome::Compared(d) => Some(d),
        }
    }
}

/// 计算两份架构报告的差异。`previous` 无数据（`None`、旧 schema、空图）时返回
/// [`ArchitectureDiffOutcome::NoBaseline`]。
pub fn architecture_diff(
    previous: Option<&ArchitectureReport>,
    current: &ArchitectureReport,
) -> ArchitectureDiffOutcome {
    let Some(prev) = previous.filter(|p| p.has_data()) else {
        return ArchitectureDiffOutcome::NoBaseline;
    };

    let prev_nodes: BTreeSet<&str> = prev.nodes.iter().map(|n| n.id.as_str()).collect();
    let cur_nodes: BTreeSet<&str> = current.nodes.iter().map(|n| n.id.as_str()).collect();
    let prev_edges: BTreeSet<&str> = prev.edges.iter().map(|e| e.id.as_str()).collect();
    let cur_edges: BTreeSet<&str> = current.edges.iter().map(|e| e.id.as_str()).collect();
    let prev_cycles: BTreeSet<&str> = prev.cycles.iter().map(|c| c.id.as_str()).collect();
    let cur_cycles: BTreeSet<&str> = current.cycles.iter().map(|c| c.id.as_str()).collect();

    ArchitectureDiffOutcome::Compared(ArchitectureDiff {
        added_nodes: set_diff(&cur_nodes, &prev_nodes),
        removed_nodes: set_diff(&prev_nodes, &cur_nodes),
        added_edges: set_diff(&cur_edges, &prev_edges),
        removed_edges: set_diff(&prev_edges, &cur_edges),
        added_cycles: set_diff(&cur_cycles, &prev_cycles),
        resolved_cycles: set_diff(&prev_cycles, &cur_cycles),
    })
}

fn set_diff(a: &BTreeSet<&str>, b: &BTreeSet<&str>) -> Vec<String> {
    a.difference(b).map(|s| s.to_string()).collect()
}

/// 影响范围中的一个受影响的内部节点及其到最近种子的最短距离。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImpactNode {
    pub node_id: String,
    pub distance: usize,
}

/// 变更影响范围：直接依赖方（距离 1）与间接影响范围（距离 2..=深度）分开。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImpactScope {
    pub direct: Vec<ImpactNode>,
    pub indirect: Vec<ImpactNode>,
    /// 因深度上限未继续展开的节点数（仅计数，不猜测）。
    pub max_distance_reached: usize,
    /// BFS 是否因访问上限被截断。
    pub truncated: bool,
}

impl ImpactScope {
    pub fn is_empty(&self) -> bool {
        self.direct.is_empty() && self.indirect.is_empty()
    }

    pub fn total(&self) -> usize {
        self.direct.len() + self.indirect.len()
    }
}

/// 影响范围计算的输入。
#[derive(Debug, Clone, Copy)]
pub struct ImpactInput<'a> {
    pub report: &'a ArchitectureReport,
    /// 相对项目根的 Git dirty 路径（规范化 `/` 分隔）。
    pub dirty_paths: &'a [String],
    /// 相对上次快照发生变化的节点 ID（如新增/移除边的端点）。
    pub changed_nodes: &'a [String],
    pub depth: usize,
    /// 访问节点总数上限，防止异常大图拖慢 UI。
    pub max_visited: usize,
}

/// 计算变更影响范围。种子来自 dirty 文件（映射到其所在节点）与显式变化节点；
/// 沿反向内部依赖边 BFS，区分直接/间接并记录最短距离。
pub fn impact_scope(input: ImpactInput<'_>) -> ImpactScope {
    let report = input.report;
    if input.depth == 0 {
        return ImpactScope::default();
    }

    let internal: HashSet<&str> = report
        .nodes
        .iter()
        .filter(|n| !n.external)
        .map(|n| n.id.as_str())
        .collect();

    // 反向邻接：to → [from]（依赖该 to 的节点即受影响方）。
    let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &report.edges {
        if !internal.contains(edge.from.as_str()) || !internal.contains(edge.to.as_str()) {
            continue;
        }
        reverse
            .entry(edge.to.as_str())
            .or_default()
            .push(edge.from.as_str());
    }

    let seeds = collect_seeds(report, input.dirty_paths, input.changed_nodes, &internal);
    if seeds.is_empty() {
        return ImpactScope::default();
    }

    // BFS：distance 从 1 起（种子自身不算受影响）。
    let mut dist: HashMap<&str, usize> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for seed in &seeds {
        queue.push_back(seed.as_str());
        dist.entry(seed.as_str()).or_insert(0);
    }

    let mut visited = 0usize;
    let mut truncated = false;
    let mut max_distance_reached = 0usize;

    while let Some(node) = queue.pop_front() {
        let d = dist[node];
        if d >= input.depth {
            max_distance_reached = max_distance_reached.max(d);
            continue;
        }
        let mut neighbours: Vec<&str> = reverse.get(node).cloned().unwrap_or_default();
        neighbours.sort_unstable();
        for next in neighbours {
            if dist.contains_key(next) {
                continue;
            }
            if visited >= input.max_visited {
                truncated = true;
                break;
            }
            dist.insert(next, d + 1);
            queue.push_back(next);
            visited += 1;
            max_distance_reached = max_distance_reached.max(d + 1);
        }
        if truncated {
            break;
        }
    }

    let mut direct: Vec<ImpactNode> = Vec::new();
    let mut indirect: Vec<ImpactNode> = Vec::new();
    for (id, distance) in dist {
        if distance == 0 {
            continue;
        }
        let item = ImpactNode {
            node_id: id.to_string(),
            distance,
        };
        if distance == 1 {
            direct.push(item);
        } else {
            indirect.push(item);
        }
    }
    direct.sort();
    indirect.sort();

    ImpactScope {
        direct,
        indirect,
        max_distance_reached,
        truncated,
    }
}

/// 收集影响种子：dirty 文件对应节点 + 显式变化节点。仅内部节点。
fn collect_seeds(
    report: &ArchitectureReport,
    dirty_paths: &[String],
    changed_nodes: &[String],
    internal: &HashSet<&str>,
) -> BTreeSet<String> {
    let mut seeds: BTreeSet<String> = BTreeSet::new();
    let dirty: BTreeSet<&str> = dirty_paths.iter().map(|s| s.as_str()).collect();

    // 文件路径 → 节点：精确匹配节点声明文件（module 的 .rs / crate 的 manifest）。
    let mut nodes_by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &report.nodes {
        if node.external {
            continue;
        }
        let Some(path) = &node.path else { continue };
        let key = normalize_path(path);
        nodes_by_path.entry(key).or_default().push(&node.id);
    }
    for path in &dirty {
        if let Some(ids) = nodes_by_path.get(path) {
            seeds.extend(ids.iter().map(|s| s.to_string()));
        }
    }

    for id in changed_nodes {
        if internal.contains(id.as_str()) {
            seeds.insert(id.clone());
        }
    }
    seeds
}

/// 归一化路径为 `/` 分隔、去前导 `./`，与 Git dirty 路径同口径。
fn normalize_path(path: &Path) -> &str {
    path.to_str().unwrap_or_default()
}

/// 从一组边的两个端点提取节点 ID（用于把边变化转成变化节点种子）。
pub fn edge_endpoints(edges: &[ArchitectureEdge]) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for edge in edges {
        set.insert(edge.from.clone());
        set.insert(edge.to.clone());
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::{
        ArchitectureEdgeKind, ArchitectureNode, ArchitectureNodeKind, cycle_id, edge_id,
    };
    use std::path::PathBuf;

    fn node(id: &str, path: Option<&str>) -> ArchitectureNode {
        ArchitectureNode {
            id: id.to_string(),
            kind: ArchitectureNodeKind::Module,
            name: id.trim_start_matches("module:").to_string(),
            qualified_name: id.trim_start_matches("module:").to_string(),
            path: path.map(PathBuf::from),
            parent_id: None,
            loc: 1,
            fan_in: 0,
            fan_out: 0,
            layer: None,
            external: false,
        }
    }

    fn edge(from: &str, to: &str) -> ArchitectureEdge {
        ArchitectureEdge {
            id: edge_id(ArchitectureEdgeKind::ModuleUse, from, to),
            from: from.to_string(),
            to: to.to_string(),
            kind: ArchitectureEdgeKind::ModuleUse,
            evidence: Vec::new(),
        }
    }

    fn report(nodes: Vec<ArchitectureNode>, edges: Vec<ArchitectureEdge>) -> ArchitectureReport {
        ArchitectureReport {
            status: crate::architecture::ArchitectureStatus::Complete,
            nodes,
            edges,
            cycles: Vec::new(),
            unresolved_edges: 0,
            errors: Vec::new(),
        }
    }

    #[test]
    fn no_baseline_when_previous_missing_or_empty() {
        let cur = report(vec![node("module:a", None)], vec![]);
        assert_eq!(
            architecture_diff(None, &cur),
            ArchitectureDiffOutcome::NoBaseline
        );
        let empty = ArchitectureReport::default();
        assert_eq!(
            architecture_diff(Some(&empty), &cur),
            ArchitectureDiffOutcome::NoBaseline
        );
    }

    #[test]
    fn added_and_removed_edges_by_stable_id_sorted() {
        let prev = report(
            vec![node("module:a", None), node("module:b", None)],
            vec![edge("module:a", "module:b")],
        );
        let cur = report(
            vec![
                node("module:a", None),
                node("module:b", None),
                node("module:c", None),
            ],
            vec![edge("module:a", "module:c"), edge("module:b", "module:c")],
        );
        let ArchitectureDiffOutcome::Compared(d) = architecture_diff(Some(&prev), &cur) else {
            panic!("应有基线");
        };
        assert_eq!(d.added_nodes, vec!["module:c"]);
        assert!(d.removed_nodes.is_empty());
        assert_eq!(
            d.added_edges,
            vec![
                edge_id(ArchitectureEdgeKind::ModuleUse, "module:a", "module:c"),
                edge_id(ArchitectureEdgeKind::ModuleUse, "module:b", "module:c"),
            ]
        );
        assert_eq!(
            d.removed_edges,
            vec![edge_id(
                ArchitectureEdgeKind::ModuleUse,
                "module:a",
                "module:b"
            )]
        );
    }

    #[test]
    fn added_and_resolved_cycles() {
        let mut prev = report(vec![node("module:a", None), node("module:b", None)], vec![]);
        prev.cycles = vec![crate::architecture::DependencyCycle {
            id: cycle_id(&["module:a".into(), "module:b".into()]),
            node_ids: vec!["module:a".into(), "module:b".into()],
            edge_ids: vec![],
        }];
        let mut cur = report(vec![node("module:a", None), node("module:c", None)], vec![]);
        cur.cycles = vec![crate::architecture::DependencyCycle {
            id: cycle_id(&["module:a".into(), "module:c".into()]),
            node_ids: vec!["module:a".into(), "module:c".into()],
            edge_ids: vec![],
        }];
        let ArchitectureDiffOutcome::Compared(d) = architecture_diff(Some(&prev), &cur) else {
            panic!("应有基线");
        };
        assert_eq!(d.added_cycles.len(), 1);
        assert_eq!(d.resolved_cycles.len(), 1);
    }

    #[test]
    fn impact_direct_and_indirect_with_shortest_distance() {
        // c → b → a（c 依赖 b，b 依赖 a）。dirty a 的影响方是 b（直接）与 c（间接）。
        let r = report(
            vec![
                node("module:a", Some("src/a.rs")),
                node("module:b", Some("src/b.rs")),
                node("module:c", Some("src/c.rs")),
            ],
            vec![edge("module:c", "module:b"), edge("module:b", "module:a")],
        );
        let dirty = vec!["src/a.rs".to_string()];
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &dirty,
            changed_nodes: &[],
            depth: 3,
            max_visited: 100,
        });
        assert_eq!(
            scope.direct,
            vec![ImpactNode {
                node_id: "module:b".into(),
                distance: 1
            }]
        );
        assert_eq!(
            scope.indirect,
            vec![ImpactNode {
                node_id: "module:c".into(),
                distance: 2
            }]
        );
    }

    #[test]
    fn impact_respects_depth_limit() {
        let r = report(
            vec![
                node("module:a", Some("src/a.rs")),
                node("module:b", None),
                node("module:c", None),
            ],
            vec![edge("module:c", "module:b"), edge("module:b", "module:a")],
        );
        let dirty = vec!["src/a.rs".to_string()];
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &dirty,
            changed_nodes: &[],
            depth: 1,
            max_visited: 100,
        });
        assert_eq!(scope.direct.len(), 1);
        assert!(scope.indirect.is_empty());
        assert_eq!(scope.max_distance_reached, 1);
    }

    #[test]
    fn impact_handles_cycle_without_infinite_loop() {
        // a ↔ b 互相依赖，从 a 出发：b 直接，a 是种子（距离 0，不重复计）。
        let r = report(
            vec![node("module:a", Some("src/a.rs")), node("module:b", None)],
            vec![edge("module:a", "module:b"), edge("module:b", "module:a")],
        );
        let dirty = vec!["src/a.rs".to_string()];
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &dirty,
            changed_nodes: &[],
            depth: 5,
            max_visited: 100,
        });
        assert_eq!(scope.total(), 1);
        assert_eq!(scope.direct[0].node_id, "module:b");
    }

    #[test]
    fn impact_from_changed_nodes_not_only_dirty() {
        let r = report(
            vec![node("module:a", None), node("module:b", None)],
            vec![edge("module:b", "module:a")],
        );
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &[],
            changed_nodes: &["module:a".to_string()],
            depth: 3,
            max_visited: 100,
        });
        assert_eq!(scope.direct.len(), 1);
        assert_eq!(scope.direct[0].node_id, "module:b");
    }

    #[test]
    fn empty_seeds_produce_empty_scope() {
        let r = report(vec![node("module:a", None)], vec![]);
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &[],
            changed_nodes: &[],
            depth: 3,
            max_visited: 100,
        });
        assert!(scope.is_empty());
    }

    #[test]
    fn max_visited_truncates_scope() {
        // 星形：hub 被 a..e 依赖，从 hub 出发有 5 个直接受影响方；上限 2 触发截断。
        let mut nodes = vec![node("module:hub", Some("src/hub.rs"))];
        for n in ["a", "b", "c", "d", "e"] {
            nodes.push(node(&format!("module:{n}"), None));
        }
        let edges: Vec<_> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|n| edge(&format!("module:{n}"), "module:hub"))
            .collect();
        let r = report(nodes, edges);
        let dirty = vec!["src/hub.rs".to_string()];
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &dirty,
            changed_nodes: &[],
            depth: 3,
            max_visited: 2,
        });
        assert!(scope.truncated);
        assert_eq!(scope.total(), 2);
    }

    #[test]
    fn external_nodes_not_in_impact() {
        let mut ext = node("external:serde", None);
        ext.external = true;
        let r = report(
            vec![node("module:a", None), node("module:b", None), ext],
            vec![
                edge("module:a", "module:b"),
                edge("module:a", "external:serde"),
            ],
        );
        let scope = impact_scope(ImpactInput {
            report: &r,
            dirty_paths: &[],
            changed_nodes: &["module:b".to_string()],
            depth: 3,
            max_visited: 100,
        });
        assert_eq!(scope.total(), 1);
        assert_eq!(scope.direct[0].node_id, "module:a");
    }
}
