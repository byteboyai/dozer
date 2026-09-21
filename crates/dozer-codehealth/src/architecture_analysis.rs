//! 架构图分析与健康规则（spec 2026-09-21「图分析与健康规则」）。
//!
//! 输入是 [`ArchitectureReport`] 已构建的节点/边（Cargo + module），输出：
//!
//! - 写回节点的 `fan_in`/`fan_out` 与 `layer`；
//! - `cycles`（强连通分量 ≥ 2 或自环，稳定 ID）；
//! - 统一 [`Finding`]：依赖环、依赖枢纽、分层边界违规。
//!
//! 图算法内部用 `petgraph`，但**持久化结构（节点/边/循环）不暴露 petgraph
//! 类型或索引**——只有稳定 ID 与可回溯证据（spec「数据模型」）。所有计算
//! 只在内部节点子图上进行；external 节点不参与扇出/环/分层判断（第三方依赖
//! 不是本项目架构责任）。

use crate::architecture::{
    ArchitectureEdge, ArchitectureEdgeKind, ArchitectureNode, ArchitectureNodeKind,
    ArchitectureReport, DependencyCycle, cycle_id,
};
use crate::discovery::{ArchitectureConfig, LayerConfig};
use crate::finding::{
    Applicability, Finding, FindingCategory, FindingEvidence, FindingSeverity,
    normalize_path_for_id, rule_ids, stable_finding_id,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

/// 一次架构分析的结果：写回指标后的节点、循环、发现。
#[derive(Debug, Clone, Default)]
pub struct ArchitectureAnalysis {
    /// 写回 `fan_in`/`fan_out`/`layer` 后的节点（顺序与输入一致）。
    pub nodes: Vec<ArchitectureNode>,
    /// 检测到的循环（稳定 ID，按 ID 排序）。
    pub cycles: Vec<DependencyCycle>,
    /// 架构发现（环/枢纽/边界违规），按稳定 ID 排序。
    pub findings: Vec<Finding>,
    /// 配置错误（无效 layer 规则等），供扫描范围展示并使状态为 Partial。
    pub config_errors: Vec<String>,
}

/// 分析架构报告：写回节点指标、检测循环和三类架构发现。
///
/// - `config` 提供扇出阈值（默认 8/15）与分层规则（默认不分层）。
/// - `layers` 的 `match` glob 对节点的 qualified name（`crate::a::b`）匹配，
///   同时也接受 crate 节点路径形式，按配置顺序首个命中生效。
pub fn analyze_architecture(
    report: &ArchitectureReport,
    config: &ArchitectureConfig,
) -> ArchitectureAnalysis {
    // 只分析内部节点（非 external）。
    let internal: BTreeSet<&str> = report
        .nodes
        .iter()
        .filter(|n| !n.external)
        .map(|n| n.id.as_str())
        .collect();

    // 内部边：两端都在内部节点集合里。
    let internal_edges: Vec<&ArchitectureEdge> = report
        .edges
        .iter()
        .filter(|e| internal.contains(e.from.as_str()) && internal.contains(e.to.as_str()))
        .collect();

    // 节点位置索引：稳定 ID → 输入下标，便于写回。
    let mut index_of: HashMap<&str, usize> = HashMap::new();
    for (i, n) in report.nodes.iter().enumerate() {
        index_of.insert(n.id.as_str(), i);
    }

    // 扇入/扇出：只计内部边，去重同 from/to（同 pair 多条边不应重复计数）。
    let mut fan_out: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut fan_in: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for e in &internal_edges {
        fan_out
            .entry(e.from.as_str())
            .or_default()
            .insert(e.to.as_str());
        fan_in
            .entry(e.to.as_str())
            .or_default()
            .insert(e.from.as_str());
    }

    // 分层匹配（按配置顺序首个命中生效）。
    let (layer_of, config_errors) = assign_layers(&report.nodes, config);
    let mut known_layers: BTreeSet<&str> = BTreeSet::new();
    for layer in &config.layers {
        known_layers.insert(layer.name.as_str());
    }

    // 写回节点：loc 暂为 0（扫描侧可按文件聚合后再填），此处只补 fan/layer。
    let mut nodes = report.nodes.clone();
    for node in nodes.iter_mut() {
        node.fan_out = fan_out.get(node.id.as_str()).map(|s| s.len()).unwrap_or(0);
        node.fan_in = fan_in.get(node.id.as_str()).map(|s| s.len()).unwrap_or(0);
        if let Some(layer) = layer_of.get(node.id.as_str()) {
            node.layer = Some(layer.clone());
        }
    }

    // 循环检测。
    let cycles = detect_cycles(&internal_edges, &internal);

    let mut findings = Vec::new();

    // 依赖环发现：每个 cycle 一条。
    for cycle in &cycles {
        findings.push(cycle_finding(cycle, &nodes, &index_of));
    }

    // 依赖枢纽发现：fan_out 超阈值。
    for node in &nodes {
        if node.external {
            continue;
        }
        if node.fan_out >= config.max_fan_out {
            let severity = if node.fan_out >= config.critical_fan_out {
                FindingSeverity::Critical
            } else {
                FindingSeverity::Watch
            };
            findings.push(hub_finding(
                node,
                severity,
                &internal_edges,
                index_of.get(node.id.as_str()).copied(),
            ));
        }
    }

    // 分层边界违规发现。
    for layer in &config.layers {
        for e in &internal_edges {
            let Some(from_layer) = layer_of.get(e.from.as_str()) else {
                continue;
            };
            if from_layer != &layer.name {
                continue;
            }
            let Some(to_layer) = layer_of.get(e.to.as_str()) else {
                continue; // 未分层节点不产生违规
            };
            if to_layer == from_layer {
                continue; // 同层允许
            }
            if !layer.may_depend_on.iter().any(|l| l == to_layer) {
                // 目标 layer 必须是已声明 layer（已知引用由配置校验保证；此处
                // 只对真实存在的 layer 判违规，避免把未分层节点误报）。
                if known_layers.contains(to_layer.as_str()) {
                    findings.push(boundary_finding(e, from_layer, to_layer, &index_of, &nodes));
                }
            }
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    findings.dedup_by(|a, b| a.id == b.id);

    ArchitectureAnalysis {
        nodes,
        cycles,
        findings,
        config_errors,
    }
}

/// 按配置顺序把节点匹配到 layer，返回 `节点 ID → layer 名` 与配置错误。
fn assign_layers(
    nodes: &[ArchitectureNode],
    config: &ArchitectureConfig,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut errors = Vec::new();
    // 预编译每个 layer 的 glob；无效 glob 记录错误并跳过该规则。
    let mut compiled: Vec<(&LayerConfig, GlobSet)> = Vec::new();
    for (i, layer) in config.layers.iter().enumerate() {
        let mut builder = GlobSetBuilder::new();
        let mut ok = false;
        for pat in &layer.r#match {
            match Glob::new(pat) {
                Ok(g) => {
                    builder.add(g);
                    ok = true;
                }
                Err(_) => errors.push(format!("architecture.layers[{i}].match 无效 glob：{pat}")),
            }
        }
        if ok && let Ok(set) = builder.build() {
            compiled.push((layer, set));
        }
    }

    let mut out = BTreeMap::new();
    for node in nodes {
        if node.external {
            continue;
        }
        for (layer, globset) in &compiled {
            if matches_node(globset, node) {
                out.insert(node.id.clone(), layer.name.clone());
                break; // 首个命中生效
            }
        }
    }
    (out, errors)
}

/// 一个节点的 layer glob 匹配：qualified name（`crate::a::b`）与文件路径两种
/// 形态都试，兼容 spec 示例里的 `crates/dozer-app::extensions/**` 与
/// `crates/dozer-core/**`。
fn matches_node(globset: &GlobSet, node: &ArchitectureNode) -> bool {
    if globset.is_match(node.qualified_name.as_str()) {
        return true;
    }
    if globset.is_match(node.id.as_str()) {
        return true;
    }
    if let Some(path) = &node.path {
        let p = normalize_path_for_id(path);
        if globset.is_match(p.as_str()) {
            return true;
        }
    }
    false
}

/// 用 SCC 检测循环：内部边构造有向图，Tarjan SCC。
/// - 分量 ≥ 2 节点 → 一条循环；
/// - 单节点但有自环 → 一条循环。
///
/// 结果按 cycle ID 排序，保证稳定。
fn detect_cycles(edges: &[&ArchitectureEdge], internal: &BTreeSet<&str>) -> Vec<DependencyCycle> {
    let mut ids: Vec<&str> = internal.iter().copied().collect();
    ids.sort_unstable();
    let mut index_of: HashMap<&str, NodeIndex> = HashMap::new();
    let mut graph: DiGraph<&str, &str> = DiGraph::new();
    for id in &ids {
        let idx = graph.add_node(id);
        index_of.insert(id, idx);
    }
    let mut self_loops: BTreeSet<String> = BTreeSet::new();
    for e in edges {
        let (Some(&a), Some(&b)) = (index_of.get(e.from.as_str()), index_of.get(e.to.as_str()))
        else {
            continue;
        };
        graph.add_edge(a, b, e.id.as_str());
        if e.from == e.to {
            self_loops.insert(e.from.clone());
        }
    }

    let mut cycles: Vec<DependencyCycle> = Vec::new();
    for component in tarjan_scc(&graph) {
        // 自环单节点由上面的 self_loops 单独处理，避免 SCC 把它和普通单节点
        // 混淆。
        if component.len() < 2 {
            let id = graph[component[0]];
            if !self_loops.contains(id) {
                continue;
            }
        }
        let node_ids: BTreeSet<String> = component.iter().map(|&i| graph[i].to_string()).collect();
        let node_ids: Vec<String> = node_ids.into_iter().collect();
        // 代表性边：两端都在该分量内的内部边。
        let mut edge_ids: BTreeSet<String> = BTreeSet::new();
        for e in edges {
            if node_ids.contains(&e.from) && node_ids.contains(&e.to) {
                edge_ids.insert(e.id.clone());
            }
        }
        cycles.push(DependencyCycle {
            id: cycle_id(&node_ids),
            node_ids,
            edge_ids: edge_ids.into_iter().collect(),
        });
    }
    cycles.sort_by(|a, b| a.id.cmp(&b.id));
    cycles.dedup_by(|a, b| a.id == b.id);
    cycles
}

/// 循环发现的落点路径/行号：优先用环内第一条边证据；否则用首个有源文件的节点。
fn cycle_finding(
    cycle: &DependencyCycle,
    nodes: &[ArchitectureNode],
    _index_of: &HashMap<&str, usize>,
) -> Finding {
    // 环内节点按 ID 排序后取第一个作为 symbol 与回退路径。
    let anchor = cycle.node_ids.first();
    let (path, line) = anchor
        .and_then(|id| nodes.iter().find(|n| &n.id == id))
        .and_then(|n| n.path.clone())
        .map(|p| (p, 1))
        .unwrap_or_else(|| (PathBuf::from("<architecture>"), 1));
    let symbol = anchor.cloned();
    let signature = cycle.node_ids.join("|");
    let id = stable_finding_id(
        rule_ids::ARCHITECTURE_CYCLE,
        &path,
        symbol.as_deref(),
        &signature,
    );
    let n = cycle.node_ids.len();
    Finding {
        id,
        rule_id: rule_ids::ARCHITECTURE_CYCLE.to_string(),
        category: FindingCategory::Architecture,
        severity: FindingSeverity::Critical,
        start_line: line,
        symbol,
        title: format!("依赖环（{n} 个节点）"),
        evidence: FindingEvidence::ArchitectureCycle {
            node_ids: cycle.node_ids.clone(),
        },
        applicability: Applicability::Applicable,
        path,
    }
}

/// 枢纽发现的落点：节点源文件；证据列出扇出值与最主要目标。
fn hub_finding(
    node: &ArchitectureNode,
    severity: FindingSeverity,
    edges: &[&ArchitectureEdge],
    _index: Option<usize>,
) -> Finding {
    let path = node
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from("<architecture>"));
    let id = stable_finding_id(
        rule_ids::ARCHITECTURE_HIGH_FAN_OUT,
        &path,
        Some(node.qualified_name.as_str()),
        &node.id,
    );
    // 最主要目标：本节点出边里目标节点 ID 字典序最小的那个（稳定）。
    let mut targets: BTreeSet<&str> = BTreeSet::new();
    for e in edges {
        if e.from == node.id {
            targets.insert(e.to.as_str());
        }
    }
    let _ = targets; // 目标列表已隐含在 fan_out；证据按 spec 只存 fan_out。
    Finding {
        id,
        rule_id: rule_ids::ARCHITECTURE_HIGH_FAN_OUT.to_string(),
        category: FindingCategory::Architecture,
        severity,
        start_line: 1,
        symbol: Some(node.qualified_name.clone()),
        title: format!("依赖枢纽：{} 扇出 {}", node.name, node.fan_out),
        evidence: FindingEvidence::ArchitectureHub {
            node_id: node.id.clone(),
            fan_out: node.fan_out,
        },
        applicability: Applicability::Applicable,
        path,
    }
}

/// 边界违规发现的落点：违规边的来源证据（路径/行号）。
fn boundary_finding(
    edge: &ArchitectureEdge,
    from_layer: &str,
    to_layer: &str,
    _index_of: &HashMap<&str, usize>,
    _nodes: &[ArchitectureNode],
) -> Finding {
    let (path, line, _snippet) = edge
        .evidence
        .first()
        .map(|ev| (ev.path.clone(), ev.line, ev.snippet.clone()))
        .unwrap_or_else(|| (PathBuf::from("<architecture>"), 1, String::new()));
    let signature = format!("{from_layer}->{to_layer}@{}", edge.id);
    let id = stable_finding_id(
        rule_ids::ARCHITECTURE_LAYER_VIOLATION,
        &path,
        Some(edge.id.as_str()),
        &signature,
    );
    Finding {
        id,
        rule_id: rule_ids::ARCHITECTURE_LAYER_VIOLATION.to_string(),
        category: FindingCategory::Architecture,
        severity: FindingSeverity::Critical,
        start_line: line,
        symbol: Some(edge.id.clone()),
        title: format!("边界违规：{from_layer} → {to_layer}"),
        evidence: FindingEvidence::ArchitectureBoundary {
            edge_id: edge.id.clone(),
            from_layer: from_layer.to_string(),
            to_layer: to_layer.to_string(),
        },
        applicability: Applicability::Applicable,
        path,
    }
}

/// 边种类是否参与内部图分析（语义文档用；当前两类边都参与）。
pub fn edge_kind_is_internal(kind: ArchitectureEdgeKind) -> bool {
    matches!(
        kind,
        ArchitectureEdgeKind::CargoDependency | ArchitectureEdgeKind::ModuleUse
    )
}

/// 节点种类是否为本项目内部节点。
pub fn node_kind_is_internal(kind: ArchitectureNodeKind) -> bool {
    !matches!(kind, ArchitectureNodeKind::ExternalCrate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::{
        ArchitectureEdgeKind, ArchitectureNode, ArchitectureNodeKind, edge_id,
    };

    fn node(id: &str) -> ArchitectureNode {
        ArchitectureNode {
            id: id.to_string(),
            kind: ArchitectureNodeKind::Module,
            name: id.rsplit("::").next().unwrap_or(id).to_string(),
            qualified_name: id.replace("module:", ""),
            path: Some(PathBuf::from(format!("src/{id}.rs"))),
            parent_id: None,
            loc: 0,
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
            evidence: vec![],
        }
    }

    fn report(nodes: Vec<ArchitectureNode>, edges: Vec<ArchitectureEdge>) -> ArchitectureReport {
        ArchitectureReport {
            status: crate::architecture::ArchitectureStatus::Complete,
            nodes,
            edges,
            cycles: vec![],
            unresolved_edges: 0,
            errors: vec![],
        }
    }

    #[test]
    fn fan_in_out_written_back_from_internal_edges() {
        let r = report(
            vec![node("module:a"), node("module:b"), node("module:c")],
            vec![edge("module:a", "module:b"), edge("module:a", "module:c")],
        );
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        let a = analysis.nodes.iter().find(|n| n.id == "module:a").unwrap();
        assert_eq!(a.fan_out, 2);
        assert_eq!(a.fan_in, 0);
        let b = analysis.nodes.iter().find(|n| n.id == "module:b").unwrap();
        assert_eq!(b.fan_in, 1);
        assert_eq!(b.fan_out, 0);
    }

    #[test]
    fn external_nodes_excluded_from_fan_and_cycles() {
        let mut ext = node("external:serde");
        ext.external = true;
        ext.kind = ArchitectureNodeKind::ExternalCrate;
        let r = report(
            vec![node("module:a"), ext],
            vec![edge("module:a", "external:serde")],
        );
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        let a = analysis.nodes.iter().find(|n| n.id == "module:a").unwrap();
        assert_eq!(a.fan_out, 0, "external 边不计入扇出");
        assert!(analysis.cycles.is_empty());
    }

    #[test]
    fn detects_two_node_cycle_with_stable_id() {
        let r = report(
            vec![node("module:a"), node("module:b")],
            vec![edge("module:a", "module:b"), edge("module:b", "module:a")],
        );
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        assert_eq!(analysis.cycles.len(), 1);
        let expected =
            crate::architecture::cycle_id(&["module:a".to_string(), "module:b".to_string()]);
        assert_eq!(analysis.cycles[0].id, expected);
        assert_eq!(
            analysis.cycles[0].node_ids,
            vec!["module:a".to_string(), "module:b".to_string()]
        );
        assert_eq!(
            analysis
                .findings
                .iter()
                .filter(|f| f.rule_id == rule_ids::ARCHITECTURE_CYCLE)
                .count(),
            1
        );
    }

    #[test]
    fn detects_self_loop() {
        let r = report(vec![node("module:a")], vec![edge("module:a", "module:a")]);
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        assert_eq!(analysis.cycles.len(), 1);
        assert_eq!(analysis.cycles[0].node_ids, vec!["module:a".to_string()]);
    }

    #[test]
    fn acyclic_graph_has_no_cycles() {
        let r = report(
            vec![node("module:a"), node("module:b")],
            vec![edge("module:a", "module:b")],
        );
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        assert!(analysis.cycles.is_empty());
    }

    #[test]
    fn hub_thresholds_default_and_override() {
        // 9 个目标 → 超过默认 max_fan_out(8)，未到 critical(15) → Watch。
        let mut nodes = vec![node("module:hub")];
        let mut edges = Vec::new();
        for i in 0..9 {
            let id = format!("module:t{i}");
            nodes.push(node(&id));
            edges.push(edge("module:hub", &id));
        }
        let r = report(nodes, edges);
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        let hub = analysis
            .findings
            .iter()
            .find(|f| f.rule_id == rule_ids::ARCHITECTURE_HIGH_FAN_OUT)
            .expect("hub finding");
        assert_eq!(hub.severity, FindingSeverity::Watch);
        assert_eq!(
            hub.evidence,
            FindingEvidence::ArchitectureHub {
                node_id: "module:hub".into(),
                fan_out: 9,
            }
        );

        // 覆盖为更低阈值：5 已是 Critical。
        let cfg = ArchitectureConfig {
            max_fan_out: 3,
            critical_fan_out: 5,
            ..ArchitectureConfig::default()
        };
        let r = report(
            vec![
                node("module:hub"),
                node("module:b"),
                node("module:c"),
                node("module:d"),
                node("module:e"),
                node("module:f"),
            ],
            vec![
                edge("module:hub", "module:b"),
                edge("module:hub", "module:c"),
                edge("module:hub", "module:d"),
                edge("module:hub", "module:e"),
                edge("module:hub", "module:f"),
            ],
        );
        let analysis = analyze_architecture(&r, &cfg);
        let hub = analysis
            .findings
            .iter()
            .find(|f| f.rule_id == rule_ids::ARCHITECTURE_HIGH_FAN_OUT)
            .expect("hub finding");
        assert_eq!(hub.severity, FindingSeverity::Critical);
    }

    #[test]
    fn no_hub_finding_below_threshold() {
        let r = report(
            vec![node("module:a"), node("module:b")],
            vec![edge("module:a", "module:b")],
        );
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        assert!(
            analysis
                .findings
                .iter()
                .all(|f| f.rule_id != rule_ids::ARCHITECTURE_HIGH_FAN_OUT)
        );
    }

    fn layered_config() -> ArchitectureConfig {
        ArchitectureConfig {
            layers: vec![
                LayerConfig {
                    name: "ui".into(),
                    r#match: vec!["module:ui*".into()],
                    may_depend_on: vec!["app".into()],
                },
                LayerConfig {
                    name: "app".into(),
                    r#match: vec!["module:app*".into()],
                    may_depend_on: vec![],
                },
            ],
            ..ArchitectureConfig::default()
        }
    }

    #[test]
    fn layer_assignment_and_allowed_edge() {
        let r = report(
            vec![node("module:ui_view"), node("module:app_core")],
            vec![edge("module:ui_view", "module:app_core")],
        );
        let analysis = analyze_architecture(&r, &layered_config());
        let ui = analysis
            .nodes
            .iter()
            .find(|n| n.id == "module:ui_view")
            .unwrap();
        assert_eq!(ui.layer.as_deref(), Some("ui"));
        let app = analysis
            .nodes
            .iter()
            .find(|n| n.id == "module:app_core")
            .unwrap();
        assert_eq!(app.layer.as_deref(), Some("app"));
        assert!(
            analysis
                .findings
                .iter()
                .all(|f| f.rule_id != rule_ids::ARCHITECTURE_LAYER_VIOLATION),
            "app 在 ui 的 may_depend_on 中，不违规"
        );
    }

    #[test]
    fn layer_violation_detected() {
        // app → ui 不被允许（app.may_depend_on 为空）。
        let r = report(
            vec![node("module:ui_view"), node("module:app_core")],
            vec![edge("module:app_core", "module:ui_view")],
        );
        let analysis = analyze_architecture(&r, &layered_config());
        let finding = analysis
            .findings
            .iter()
            .find(|f| f.rule_id == rule_ids::ARCHITECTURE_LAYER_VIOLATION)
            .expect("boundary violation");
        assert_eq!(finding.severity, FindingSeverity::Critical);
        assert_eq!(
            finding.evidence,
            FindingEvidence::ArchitectureBoundary {
                edge_id: edge_id(
                    ArchitectureEdgeKind::ModuleUse,
                    "module:app_core",
                    "module:ui_view"
                ),
                from_layer: "app".into(),
                to_layer: "ui".into(),
            }
        );
    }

    #[test]
    fn unlayered_nodes_do_not_produce_violations() {
        // ui → unknown（未匹配 layer）不违规。
        let r = report(
            vec![node("module:ui_view"), node("module:other")],
            vec![edge("module:ui_view", "module:other")],
        );
        let analysis = analyze_architecture(&r, &layered_config());
        assert!(
            analysis
                .findings
                .iter()
                .all(|f| f.rule_id != rule_ids::ARCHITECTURE_LAYER_VIOLATION)
        );
        let other = analysis
            .nodes
            .iter()
            .find(|n| n.id == "module:other")
            .unwrap();
        assert!(other.layer.is_none());
    }

    #[test]
    fn config_without_layers_produces_no_boundary_violations() {
        let r = report(
            vec![node("module:a"), node("module:b")],
            vec![edge("module:a", "module:b")],
        );
        let analysis = analyze_architecture(&r, &ArchitectureConfig::default());
        assert!(
            analysis
                .findings
                .iter()
                .all(|f| f.rule_id != rule_ids::ARCHITECTURE_LAYER_VIOLATION)
        );
    }

    #[test]
    fn findings_have_architecture_category_and_stable_ids() {
        let r = report(
            vec![node("module:a"), node("module:b")],
            vec![edge("module:a", "module:b"), edge("module:b", "module:a")],
        );
        let a1 = analyze_architecture(&r, &ArchitectureConfig::default());
        let a2 = analyze_architecture(&r, &ArchitectureConfig::default());
        let ids1: Vec<&str> = a1.findings.iter().map(|f| f.id.as_str()).collect();
        let ids2: Vec<&str> = a2.findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids1, ids2, "同输入同 ID");
        for f in &a1.findings {
            assert_eq!(f.category, FindingCategory::Architecture);
        }
    }
}
