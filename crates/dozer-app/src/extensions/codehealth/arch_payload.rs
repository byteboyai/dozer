//! 架构页 payload:把 `ProjectReport.architecture`、架构差异、影响范围与架构类
//! 发现项算成前端可直接渲染的 DTO。风险分析(循环/枢纽/越界)、差异、影响范围
//! 全部在 Rust 里算好;前端只做可见子图投影与渲染。

use super::WorkspaceState;
use super::protocol::{FindingRowDto, row_dto};
use dozer_codehealth::{
    ArchitectureDiffOutcome, ArchitectureEdgeKind, ArchitectureNodeKind, ArchitectureStatus,
    FindingEvidence, ProjectReport,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// 超过这个节点数就只发 crate 层(模块图太大,前端不遍历完整图)。
pub const MAX_PAYLOAD_NODES: usize = 3000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchNodeDto {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub qualified_name: String,
    pub path: Option<String>,
    pub parent_id: Option<String>,
    pub loc: usize,
    pub fan_in: usize,
    pub fan_out: usize,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchEdgeDto {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub evidence_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchCycleDto {
    pub id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchRiskDto {
    /// `cycle` | `hub` | `boundary`。
    pub kind: &'static str,
    pub finding: FindingRowDto,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchDiffDto {
    /// `no_baseline` | `unavailable` | `compared`。
    pub state: &'static str,
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
    pub added_cycles: Vec<String>,
    pub resolved_cycles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchImpactNodeDto {
    pub node_id: String,
    pub distance: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchImpactDto {
    pub direct: Vec<ArchImpactNodeDto>,
    pub indirect: Vec<ArchImpactNodeDto>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArchitectureBody {
    /// `complete` | `partial` | `not_applicable`。
    pub status: &'static str,
    pub status_note: Option<String>,
    pub truncated_note: Option<String>,
    pub nodes: Vec<ArchNodeDto>,
    pub edges: Vec<ArchEdgeDto>,
    pub cycles: Vec<ArchCycleDto>,
    pub risks: Vec<ArchRiskDto>,
    pub diff: ArchDiffDto,
    pub impact: ArchImpactDto,
    pub errors: Vec<String>,
    pub unresolved_edges: usize,
}

fn node_kind(k: ArchitectureNodeKind) -> &'static str {
    match k {
        ArchitectureNodeKind::Workspace => "workspace",
        ArchitectureNodeKind::Crate => "crate",
        ArchitectureNodeKind::Module => "module",
        ArchitectureNodeKind::ExternalCrate => "external_crate",
    }
}

fn edge_kind(k: ArchitectureEdgeKind) -> &'static str {
    match k {
        ArchitectureEdgeKind::CargoDependency => "cargo_dependency",
        ArchitectureEdgeKind::ModuleUse => "module_use",
    }
}

fn diff_dto(outcome: &ArchitectureDiffOutcome) -> ArchDiffDto {
    let empty = ArchDiffDto {
        state: "no_baseline",
        added_nodes: vec![],
        removed_nodes: vec![],
        added_edges: vec![],
        removed_edges: vec![],
        added_cycles: vec![],
        resolved_cycles: vec![],
    };
    match outcome {
        ArchitectureDiffOutcome::NoBaseline => empty,
        ArchitectureDiffOutcome::Unavailable => ArchDiffDto {
            state: "unavailable",
            ..empty
        },
        ArchitectureDiffOutcome::Compared(d) => ArchDiffDto {
            state: "compared",
            added_nodes: d.added_nodes.clone(),
            removed_nodes: d.removed_nodes.clone(),
            added_edges: d.added_edges.clone(),
            removed_edges: d.removed_edges.clone(),
            added_cycles: d.added_cycles.clone(),
            resolved_cycles: d.resolved_cycles.clone(),
        },
    }
}

pub fn architecture_body(ws: &WorkspaceState, report: &ProjectReport) -> ArchitectureBody {
    let arch = &report.architecture;
    let (status, status_note) = match arch.status {
        ArchitectureStatus::Complete => ("complete", None),
        ArchitectureStatus::Partial => (
            "partial",
            Some("架构分析不完整(见扫描范围或下方错误),不能据此判断没有风险。".to_string()),
        ),
        ArchitectureStatus::NotApplicable => (
            "not_applicable",
            Some(
                "没有可用的架构数据(旧版报告或项目内没有可分析的 Rust 代码),请重新扫描。"
                    .to_string(),
            ),
        ),
    };

    // 外部依赖 crate 默认隐藏:节点与触及它们的边一并丢弃。
    let external: BTreeSet<&str> = arch
        .nodes
        .iter()
        .filter(|n| n.external || n.kind == ArchitectureNodeKind::ExternalCrate)
        .map(|n| n.id.as_str())
        .collect();
    let mut kept_nodes: Vec<&dozer_codehealth::ArchitectureNode> = arch
        .nodes
        .iter()
        .filter(|n| !external.contains(n.id.as_str()))
        .collect();

    let mut truncated_note = None;
    if kept_nodes.len() > MAX_PAYLOAD_NODES {
        let total = kept_nodes.len();
        kept_nodes.retain(|n| n.kind == ArchitectureNodeKind::Crate);
        truncated_note = Some(format!(
            "模块图过大({total} 个节点),仅显示 crate 层;分析结果(循环/枢纽/越界)仍基于完整图。"
        ));
    }
    let kept_ids: BTreeSet<&str> = kept_nodes.iter().map(|n| n.id.as_str()).collect();

    let nodes: Vec<ArchNodeDto> = kept_nodes
        .iter()
        .map(|n| ArchNodeDto {
            id: n.id.clone(),
            kind: node_kind(n.kind),
            name: n.name.clone(),
            qualified_name: n.qualified_name.clone(),
            path: n.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            parent_id: n.parent_id.clone(),
            loc: n.loc,
            fan_in: n.fan_in,
            fan_out: n.fan_out,
            layer: n.layer.clone(),
        })
        .collect();

    // 全部(含被裁掉的)边的端点表,供风险映射用。
    let endpoints: BTreeMap<&str, (&str, &str)> = arch
        .edges
        .iter()
        .map(|e| (e.id.as_str(), (e.from.as_str(), e.to.as_str())))
        .collect();

    let edges: Vec<ArchEdgeDto> = arch
        .edges
        .iter()
        .filter(|e| kept_ids.contains(e.from.as_str()) && kept_ids.contains(e.to.as_str()))
        .map(|e| ArchEdgeDto {
            id: e.id.clone(),
            from: e.from.clone(),
            to: e.to.clone(),
            kind: edge_kind(e.kind),
            evidence_count: e.evidence.len(),
        })
        .collect();

    let cycles = arch
        .cycles
        .iter()
        .map(|c| ArchCycleDto {
            id: c.id.clone(),
            node_ids: c.node_ids.clone(),
            edge_ids: c.edge_ids.clone(),
        })
        .collect();

    let mut risks = Vec::new();
    for f in report
        .findings
        .iter()
        .filter(|f| matches!(f.category, dozer_codehealth::FindingCategory::Architecture))
    {
        let (kind, node_ids, edge_ids): (&'static str, Vec<String>, Vec<String>) = match &f.evidence
        {
            FindingEvidence::ArchitectureCycle { node_ids } => {
                let set: BTreeSet<&str> = node_ids.iter().map(String::as_str).collect();
                let eids = arch
                    .edges
                    .iter()
                    .filter(|e| set.contains(e.from.as_str()) && set.contains(e.to.as_str()))
                    .map(|e| e.id.clone())
                    .collect();
                ("cycle", node_ids.clone(), eids)
            }
            FindingEvidence::ArchitectureHub { node_id, .. } => {
                ("hub", vec![node_id.clone()], Vec::new())
            }
            FindingEvidence::ArchitectureBoundary { edge_id, .. } => {
                let nids = endpoints
                    .get(edge_id.as_str())
                    .map(|(a, b)| vec![a.to_string(), b.to_string()])
                    .unwrap_or_default();
                ("boundary", nids, vec![edge_id.clone()])
            }
            _ => continue,
        };
        risks.push(ArchRiskDto {
            kind,
            finding: row_dto(
                f.id.clone(),
                f.title.clone(),
                &f.path,
                f.start_line,
                f.severity,
                None,
                Vec::new(),
            ),
            node_ids,
            edge_ids,
        });
    }

    let impact = ws.impact();
    let to_dto = |v: &[dozer_codehealth::ImpactNode]| {
        v.iter()
            .map(|n| ArchImpactNodeDto {
                node_id: n.node_id.clone(),
                distance: n.distance,
            })
            .collect::<Vec<_>>()
    };

    ArchitectureBody {
        status,
        status_note,
        truncated_note,
        nodes,
        edges,
        cycles,
        risks,
        diff: diff_dto(ws.architecture_diff()),
        impact: ArchImpactDto {
            direct: to_dto(&impact.direct),
            indirect: to_dto(&impact.indirect),
            truncated: impact.truncated,
        },
        errors: arch.errors.clone(),
        unresolved_edges: arch.unresolved_edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{
        ArchitectureEdge, ArchitectureNode, ArchitectureReport, DependencyCycle, Finding,
        FindingCategory, FindingSeverity, HealthTier, SCHEMA_VERSION, ScanMetadata, ScanStatus,
        rule_ids,
    };
    use std::path::PathBuf;

    fn node(id: &str, kind: ArchitectureNodeKind, parent: Option<&str>) -> ArchitectureNode {
        ArchitectureNode {
            id: id.into(),
            kind,
            name: id.rsplit(':').next().unwrap_or(id).into(),
            qualified_name: id
                .trim_start_matches("module:")
                .trim_start_matches("crate:")
                .into(),
            path: Some(PathBuf::from("src/x.rs")),
            parent_id: parent.map(str::to_owned),
            loc: 10,
            fan_in: 1,
            fan_out: 2,
            layer: None,
            external: kind == ArchitectureNodeKind::ExternalCrate,
        }
    }

    fn edge(kind: ArchitectureEdgeKind, from: &str, to: &str) -> ArchitectureEdge {
        ArchitectureEdge {
            id: dozer_codehealth::architecture::edge_id(kind, from, to),
            from: from.into(),
            to: to.into(),
            kind,
            evidence: vec![],
        }
    }

    fn report_with(arch: ArchitectureReport, findings: Vec<Finding>) -> ProjectReport {
        ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                status: ScanStatus::Complete,
                analyzed_files: 1,
                ..ScanMetadata::default()
            },
            git: None,
            findings,
            architecture: arch,
            total_loc: 1,
            total_functions: 1,
            critical_functions: 0,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Healthy,
            overall_tier: HealthTier::Healthy,
            functions: vec![],
            color_findings: vec![],
            color_tier: HealthTier::Healthy,
            spacing_findings: vec![],
            spacing_tier: HealthTier::Healthy,
            font_findings: vec![],
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: vec![],
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }

    fn ws_with(report: ProjectReport) -> WorkspaceState {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Loaded(
                1,
                Box::new(super::super::PanelState {
                    report: Some(report),
                    ..Default::default()
                }),
            ),
        );
        ws
    }

    fn arch(nodes: Vec<ArchitectureNode>, edges: Vec<ArchitectureEdge>) -> ArchitectureReport {
        ArchitectureReport {
            status: ArchitectureStatus::Complete,
            nodes,
            edges,
            cycles: vec![],
            unresolved_edges: 0,
            errors: vec![],
        }
    }

    fn cycle_finding(ids: &[&str]) -> Finding {
        Finding {
            id: "arch-cycle-1".into(),
            rule_id: rule_ids::ARCHITECTURE_CYCLE.into(),
            category: FindingCategory::Architecture,
            severity: FindingSeverity::Critical,
            path: PathBuf::from("src/a.rs"),
            start_line: 1,
            symbol: None,
            title: "循环依赖".into(),
            evidence: FindingEvidence::ArchitectureCycle {
                node_ids: ids.iter().map(|s| s.to_string()).collect(),
            },
            applicability: Default::default(),
        }
    }

    #[test]
    fn not_applicable_report_is_not_zero_risk() {
        let ws = ws_with(report_with(ArchitectureReport::not_applicable(), vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.status, "not_applicable");
        assert!(b.status_note.as_deref().unwrap().contains("重新扫描"));
        assert!(b.nodes.is_empty());
    }

    #[test]
    fn external_crates_and_their_edges_are_dropped() {
        let nodes = vec![
            node("crate:a", ArchitectureNodeKind::Crate, None),
            node("crate:b", ArchitectureNodeKind::Crate, None),
            node("external:serde", ArchitectureNodeKind::ExternalCrate, None),
        ];
        let edges = vec![
            edge(ArchitectureEdgeKind::CargoDependency, "crate:a", "crate:b"),
            edge(
                ArchitectureEdgeKind::CargoDependency,
                "crate:a",
                "external:serde",
            ),
        ];
        let ws = ws_with(report_with(arch(nodes, edges), vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.status, "complete");
        assert_eq!(b.nodes.len(), 2);
        assert_eq!(b.edges.len(), 1);
        assert_eq!(b.edges[0].to, "crate:b");
        assert_eq!(b.edges[0].kind, "cargo_dependency");
    }

    #[test]
    fn cycle_finding_becomes_risk_with_node_and_edge_ids() {
        let nodes = vec![
            node("module:a::x", ArchitectureNodeKind::Module, Some("crate:a")),
            node("module:a::y", ArchitectureNodeKind::Module, Some("crate:a")),
        ];
        let edges = vec![
            edge(
                ArchitectureEdgeKind::ModuleUse,
                "module:a::x",
                "module:a::y",
            ),
            edge(
                ArchitectureEdgeKind::ModuleUse,
                "module:a::y",
                "module:a::x",
            ),
        ];
        let ws = ws_with(report_with(
            arch(nodes, edges.clone()),
            vec![cycle_finding(&["module:a::x", "module:a::y"])],
        ));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.risks.len(), 1);
        let r = &b.risks[0];
        assert_eq!(r.kind, "cycle");
        assert_eq!(r.node_ids, vec!["module:a::x", "module:a::y"]);
        let mut got = r.edge_ids.clone();
        got.sort();
        let mut want: Vec<String> = edges.iter().map(|e| e.id.clone()).collect();
        want.sort();
        assert_eq!(got, want, "环内两条边都应归入风险的 edge_ids");
        assert_eq!(r.finding.title, "循环依赖");
    }

    #[test]
    fn hub_and_boundary_findings_map_to_risks() {
        let nodes = vec![
            node("module:a::x", ArchitectureNodeKind::Module, Some("crate:a")),
            node("module:a::y", ArchitectureNodeKind::Module, Some("crate:a")),
        ];
        let e = edge(
            ArchitectureEdgeKind::ModuleUse,
            "module:a::x",
            "module:a::y",
        );
        let hub = Finding {
            id: "hub".into(),
            rule_id: rule_ids::ARCHITECTURE_HIGH_FAN_OUT.into(),
            evidence: FindingEvidence::ArchitectureHub {
                node_id: "module:a::x".into(),
                fan_out: 30,
            },
            ..cycle_finding(&[])
        };
        let boundary = Finding {
            id: "bd".into(),
            rule_id: rule_ids::ARCHITECTURE_LAYER_VIOLATION.into(),
            evidence: FindingEvidence::ArchitectureBoundary {
                edge_id: e.id.clone(),
                from_layer: "ui".into(),
                to_layer: "core".into(),
            },
            ..cycle_finding(&[])
        };
        let ws = ws_with(report_with(
            arch(nodes, vec![e.clone()]),
            vec![hub, boundary],
        ));
        let b = architecture_body(&ws, ws.report().unwrap());
        let kinds: BTreeSet<&str> = b.risks.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, BTreeSet::from(["hub", "boundary"]));
        let boundary = b.risks.iter().find(|r| r.kind == "boundary").unwrap();
        assert_eq!(boundary.edge_ids, vec![e.id.clone()]);
        assert_eq!(boundary.node_ids, vec!["module:a::x", "module:a::y"]);
        let hub = b.risks.iter().find(|r| r.kind == "hub").unwrap();
        assert_eq!(hub.node_ids, vec!["module:a::x"]);
    }

    /// Review Focus 5:大图裁成 crate 层,并带截断说明。
    #[test]
    fn oversized_graph_is_cut_to_crate_layer_with_note() {
        let mut nodes = vec![
            node("crate:a", ArchitectureNodeKind::Crate, None),
            node("crate:b", ArchitectureNodeKind::Crate, None),
        ];
        for i in 0..(MAX_PAYLOAD_NODES + 5) {
            nodes.push(node(
                &format!("module:a::m{i}"),
                ArchitectureNodeKind::Module,
                Some("crate:a"),
            ));
        }
        let edges = vec![
            edge(ArchitectureEdgeKind::CargoDependency, "crate:a", "crate:b"),
            edge(
                ArchitectureEdgeKind::ModuleUse,
                "module:a::m0",
                "module:a::m1",
            ),
        ];
        let ws = ws_with(report_with(arch(nodes, edges), vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert!(b.nodes.iter().all(|n| n.kind == "crate"), "只保留 crate 层");
        assert_eq!(b.edges.len(), 1);
        assert!(b.truncated_note.as_deref().unwrap().contains("crate"));
    }

    #[test]
    fn partial_status_carries_errors_and_note() {
        let mut a = arch(
            vec![node("crate:a", ArchitectureNodeKind::Crate, None)],
            vec![],
        );
        a.status = ArchitectureStatus::Partial;
        a.errors = vec!["cargo metadata 失败".into()];
        a.cycles = vec![DependencyCycle {
            id: "c".into(),
            node_ids: vec![],
            edge_ids: vec![],
        }];
        let ws = ws_with(report_with(a, vec![]));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.status, "partial");
        assert_eq!(b.errors, vec!["cargo metadata 失败"]);
        assert!(b.status_note.is_some());
    }

    #[test]
    fn no_baseline_diff_is_not_rendered_as_all_new() {
        let ws = ws_with(report_with(
            arch(
                vec![node("crate:a", ArchitectureNodeKind::Crate, None)],
                vec![],
            ),
            vec![],
        ));
        let b = architecture_body(&ws, ws.report().unwrap());
        assert_eq!(b.diff.state, "no_baseline");
        assert!(b.diff.added_nodes.is_empty() && b.diff.added_edges.is_empty());
    }

    #[test]
    fn payload_serializes_stable_shape() {
        let ws = ws_with(report_with(
            arch(
                vec![node("crate:a", ArchitectureNodeKind::Crate, None)],
                vec![],
            ),
            vec![],
        ));
        let v = serde_json::to_value(architecture_body(&ws, ws.report().unwrap())).unwrap();
        assert_eq!(v["status"], "complete");
        assert_eq!(v["nodes"][0]["id"], "crate:a");
        assert_eq!(v["nodes"][0]["kind"], "crate");
        assert!(v["diff"]["state"].is_string());
        assert!(v["impact"]["direct"].is_array());
    }
}
