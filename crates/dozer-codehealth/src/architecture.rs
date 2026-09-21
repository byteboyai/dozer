//! schema v3 架构模型：与具体图库无关、可 serde 的架构节点/边/证据/循环。
//!
//! 这些类型是扫描产物的持久化形状（spec
//! docs/superpowers/specs/2026-09-21-code-health-architecture-map-design.md
//! 「数据模型」）。图算法（SCC/扇入扇出/BFS）的内部索引结构不得泄漏到这里——
//! 持久化只保留稳定 ID 与可回溯证据。
//!
//! 稳定 ID 规则（spec「稳定标识」）：
//!
//! - crate 节点：`crate:<Cargo package name>`；同名 workspace package 再加
//!   规范化 manifest 相对路径。
//! - module 节点：`module:<crate name>::<Rust module path>`。
//! - external crate 节点：`external:<package name>`。
//! - 边：`edge:<kind>:<from>:<to>`（不含行号与证据数量）。
//! - cycle：对环内节点 ID 排序后做稳定哈希，不依赖 DFS 返回顺序。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 架构分析的完整度状态。
///
/// 旧 schema v2 报告缺 `architecture` 字段时回落 [`ArchitectureStatus::NotApplicable`]，
/// 避免旧报告被 UI 解读成“完整且零风险”（spec「数据模型」「错误处理」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureStatus {
    /// 分析完整：Cargo 与 module 图均成功构建。
    Complete,
    /// 部分完成：Cargo metadata 失败、个别文件解析失败、配置无效或图被截断。
    Partial,
    /// 不适用/无基线：旧报告或项目内没有可分析的 Rust 代码。
    #[default]
    NotApplicable,
}

/// 架构节点种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureNodeKind {
    /// workspace 根（可选展示）。
    Workspace,
    /// workspace member crate。
    Crate,
    /// crate 内的 Rust module（含文件 module 与内联 module）。
    Module,
    /// 直接第三方依赖 crate（默认在 UI 中隐藏）。
    ExternalCrate,
}

/// 架构边种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureEdgeKind {
    /// Cargo.toml 中声明的 workspace member 依赖。
    CargoDependency,
    /// Rust `use` 语句证明的 module 依赖。
    ModuleUse,
}

/// 一条可回溯的依赖证据。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArchitectureEvidence {
    /// 相对项目根的规范化路径。
    pub path: PathBuf,
    /// 1-based 行号。Cargo 依赖证据用 manifest 行（若可定位），否则 1。
    pub line: usize,
    /// 短片段（use 语句或依赖声明文本）。
    pub snippet: String,
}

/// 一个架构节点。`id` 为稳定标识（见模块头）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureNode {
    pub id: String,
    pub kind: ArchitectureNodeKind,
    /// 短名（crate 名 / module 段名）。
    pub name: String,
    /// 限定名（crate::a::b 或 package 名）。
    pub qualified_name: String,
    /// 声明文件（crate root / module 文件）；external crate 为 `None`。
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// 父节点的稳定 ID；workspace 根为 `None`。
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 节点自身代码行数（crate/module 聚合；external 为 0）。
    #[serde(default)]
    pub loc: usize,
    /// 扇入/扇出（由图算法写回；external 也统计）。
    #[serde(default)]
    pub fan_in: usize,
    #[serde(default)]
    pub fan_out: usize,
    /// 匹配到的 layer 名（由分层配置决定；未匹配为 `None`）。
    #[serde(default)]
    pub layer: Option<String>,
    /// 是否第三方 crate。第三方默认不在 UI 中展示。
    #[serde(default)]
    pub external: bool,
}

/// 一条聚合后的依赖边。多条同 from/to 的 use 聚合为一条，证据列表去重稳定排序。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: ArchitectureEdgeKind,
    pub evidence: Vec<ArchitectureEvidence>,
}

/// 一个依赖环（强连通分量 ≥ 2 个节点，或自环）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyCycle {
    pub id: String,
    /// 环内节点 ID，按字典序排序（稳定）。
    pub node_ids: Vec<String>,
    /// 环内代表性边 ID，按字典序排序。
    pub edge_ids: Vec<String>,
}

/// 架构分析报告。与具体图库无关、可直接 serde 持久化。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureReport {
    #[serde(default)]
    pub status: ArchitectureStatus,
    #[serde(default)]
    pub nodes: Vec<ArchitectureNode>,
    #[serde(default)]
    pub edges: Vec<ArchitectureEdge>,
    #[serde(default)]
    pub cycles: Vec<DependencyCycle>,
    /// 无法可靠解析的 use/依赖数量（只计数，不猜测目标）。
    #[serde(default)]
    pub unresolved_edges: usize,
    /// 可定位的架构分析错误（Cargo metadata 失败、配置无效、图截断等）。
    /// 有非空错误时状态应为 `Partial`；旧 JSON 缺字段时回落空列表。
    #[serde(default)]
    pub errors: Vec<String>,
}

impl ArchitectureReport {
    /// 供无 Rust/Cargo 分析结果时使用的“不适用”空报告。
    pub fn not_applicable() -> Self {
        Self::default()
    }

    /// 是否携带可用架构数据（UI 据此区分“空图”与“零风险”）。
    pub fn has_data(&self) -> bool {
        !self.nodes.is_empty() || !self.edges.is_empty()
    }
}

/// 构造边稳定 ID：`edge:<kind>:<from>:<to>`。不含行号/证据数量。
pub fn edge_id(kind: ArchitectureEdgeKind, from: &str, to: &str) -> String {
    let k = match kind {
        ArchitectureEdgeKind::CargoDependency => "cargo_dependency",
        ArchitectureEdgeKind::ModuleUse => "module_use",
    };
    format!("edge:{k}:{from}:{to}")
}

/// 构造 crate 节点稳定 ID。
pub fn crate_node_id(package_name: &str) -> String {
    format!("crate:{package_name}")
}

/// 构造 external crate 节点稳定 ID。
pub fn external_node_id(package_name: &str) -> String {
    format!("external:{package_name}")
}

/// 构造 module 节点稳定 ID：`module:<crate>::<rust module path>`。
pub fn module_node_id(crate_name: &str, module_path: &str) -> String {
    if module_path.is_empty() {
        format!("module:{crate_name}")
    } else {
        format!("module:{crate_name}::{module_path}")
    }
}

/// 构造 cycle 稳定 ID：对环内节点 ID 排序后做 FNV-1a 哈希。
pub fn cycle_id(node_ids: &[String]) -> String {
    let mut sorted: Vec<&str> = node_ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for id in &sorted {
        for &b in id.as_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("cycle:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ArchitectureStatus::Partial).unwrap(),
            "\"partial\""
        );
        assert_eq!(
            serde_json::to_string(&ArchitectureStatus::NotApplicable).unwrap(),
            "\"not_applicable\""
        );
    }

    #[test]
    fn node_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ArchitectureNodeKind::ExternalCrate).unwrap(),
            "\"external_crate\""
        );
        assert_eq!(
            serde_json::to_string(&ArchitectureEdgeKind::CargoDependency).unwrap(),
            "\"cargo_dependency\""
        );
    }

    #[test]
    fn default_report_status_is_not_applicable() {
        let report = ArchitectureReport::default();
        assert_eq!(report.status, ArchitectureStatus::NotApplicable);
        assert!(!report.has_data());
    }

    #[test]
    fn empty_report_round_trips() {
        let report = ArchitectureReport::default();
        let json = serde_json::to_string(&report).unwrap();
        let back: ArchitectureReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn full_report_round_trips() {
        let report = ArchitectureReport {
            status: ArchitectureStatus::Complete,
            nodes: vec![ArchitectureNode {
                id: crate_node_id("dozer-core"),
                kind: ArchitectureNodeKind::Crate,
                name: "dozer-core".into(),
                qualified_name: "dozer-core".into(),
                path: Some(PathBuf::from("crates/dozer-core/Cargo.toml")),
                parent_id: None,
                loc: 10,
                fan_in: 1,
                fan_out: 2,
                layer: Some("shared".into()),
                external: false,
            }],
            edges: vec![ArchitectureEdge {
                id: edge_id(ArchitectureEdgeKind::ModuleUse, "module:a", "module:b"),
                from: "module:a".into(),
                to: "module:b".into(),
                kind: ArchitectureEdgeKind::ModuleUse,
                evidence: vec![ArchitectureEvidence {
                    path: PathBuf::from("src/a.rs"),
                    line: 3,
                    snippet: "use crate::b;".into(),
                }],
            }],
            cycles: vec![DependencyCycle {
                id: cycle_id(&["module:a".into(), "module:b".into()]),
                node_ids: vec!["module:a".into(), "module:b".into()],
                edge_ids: vec!["edge:module_use:module:a:module:b".into()],
            }],
            unresolved_edges: 2,
            errors: vec!["cargo metadata 失败：boom".into()],
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: ArchitectureReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn cycle_id_is_order_independent() {
        let a = cycle_id(&["module:a".into(), "module:b".into()]);
        let b = cycle_id(&["module:b".into(), "module:a".into()]);
        assert_eq!(a, b);
    }

    #[test]
    fn edge_id_has_no_line_or_evidence() {
        let id = edge_id(ArchitectureEdgeKind::ModuleUse, "module:a", "module:b");
        assert_eq!(id, "edge:module_use:module:a:module:b");
    }
}
