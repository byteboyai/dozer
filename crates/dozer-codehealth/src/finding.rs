//! 统一发现项外壳与稳定 ID 计算（spec 2026-09-21 代码健康度演进）。
//!
//! 不同类别的静态问题（结构复杂度、颜色/边距/字体硬编码、嵌套深度、
//! 回调密度、重复结构）统一成同一种 [`Finding`]，便于跨扫描计算差异、
//! 交给 agent 分析，以及按规则分组渲染。
//!
//! 稳定 ID 是发现项的跨扫描身份：只由 `rule_id + 规范化相对路径 + symbol
//! + 规则相关结构签名` 计算，**不**包含行号、当前严重度或当前指标值——
//!
//! 函数内新增几行不会被误判成“旧问题消失且新问题产生”（spec「统一发现项」）。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 发现项所属的展示分类（对应 UI 右侧分类导航的“结构复杂度 / UI 一致性”）。
/// 具体是哪一条规则由 [`Finding::rule_id`] 区分，这里只做粗分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Structure,
    UiConsistency,
    /// 架构风险：依赖环、依赖枢纽、分层边界违规（spec「统一发现与差异」）。
    Architecture,
}

/// 发现项严重度。统一外壳只保留真正构成问题的两档（Watch/Critical）；
/// “健康/无问题”的东西根本不产出发现项，故没有 Normal/Healthy 一档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Watch,
    Critical,
}

/// 这条发现是否适用于当前项目。UI 一致性规则只对 iced/Rust 生效；未识别到
/// 适用框架时标记 `NotApplicable`（不得映射成“健康”），见 spec「UI 一致性」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    #[default]
    Applicable,
    NotApplicable,
}

/// 规则相关证据。差异计算与热点排序里“同 ID 的规则证据决定改善或恶化”
/// 靠 [`FindingEvidence::metric_value`]：数值越大越糟；字面量发现没有
/// 可比的单值指标，返回 `None`（跨扫描只按 ID 存在性判定新增/已解决）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindingEvidence {
    /// 结构复杂度：控制流信号（越高越糟）+ 函数行数 + UI 侧两个粗代理。
    Structure {
        complexity_signal: usize,
        loc: usize,
        widget_nesting_depth: usize,
        event_handler_count: usize,
    },
    NestingDepth {
        depth: usize,
    },
    EventHandlers {
        count: usize,
    },
    /// 字面量硬编码（颜色/边距/字体）：代码片段即身份，没有数值指标。
    Literal {
        snippet: String,
    },
    /// 组件化重复结构：同结构出现次数。
    Duplicate {
        occurrences: usize,
    },
    /// 架构：依赖环（强连通分量或自环）。环内节点 ID 稳定排序。
    ArchitectureCycle {
        node_ids: Vec<String>,
    },
    /// 架构：依赖枢纽（扇出超阈值）。
    ArchitectureHub {
        node_id: String,
        fan_out: usize,
    },
    /// 架构：分层边界违规（from 依赖了 may_depend_on 之外的 layer）。
    ArchitectureBoundary {
        edge_id: String,
        from_layer: String,
        to_layer: String,
    },
}

impl FindingEvidence {
    /// 差异/热点排序用的数值指标：越大越糟。字面量类返回 `None`。
    pub fn metric_value(&self) -> Option<i64> {
        match self {
            FindingEvidence::Structure {
                complexity_signal, ..
            } => Some(*complexity_signal as i64),
            FindingEvidence::NestingDepth { depth } => Some(*depth as i64),
            FindingEvidence::EventHandlers { count } => Some(*count as i64),
            FindingEvidence::Duplicate { occurrences } => Some(*occurrences as i64),
            FindingEvidence::Literal { .. } => None,
            // hub 的线性指标是扇出；cycle / boundary 没有单一可比数值，返回
            // `None`（跨扫描只按 ID 存在性判定新增/已解决，spec「统一发现与差异」）。
            FindingEvidence::ArchitectureHub { fan_out, .. } => Some(*fan_out as i64),
            FindingEvidence::ArchitectureCycle { .. }
            | FindingEvidence::ArchitectureBoundary { .. } => None,
        }
    }
}

/// 统一发现项。`id` 是跨扫描稳定身份（见模块头注释）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub rule_id: String,
    pub category: FindingCategory,
    pub severity: FindingSeverity,
    /// 相对项目根目录的规范化路径。
    pub path: std::path::PathBuf,
    pub start_line: usize,
    #[serde(default)]
    pub symbol: Option<String>,
    pub title: String,
    pub evidence: FindingEvidence,
    #[serde(default)]
    pub applicability: Applicability,
}

/// 各规则的 `rule_id` 常量，统一收口避免散落魔法字符串。
pub mod rule_ids {
    pub const STRUCTURE_COMPLEXITY: &str = "structure/complexity_signal";
    pub const COLOR_HARDCODE: &str = "ui/color_hardcode";
    pub const SPACING_HARDCODE: &str = "ui/spacing_hardcode";
    pub const FONT_HARDCODE: &str = "ui/font_hardcode";
    pub const NESTING_DEPTH: &str = "ui/nesting_depth";
    pub const EVENT_HANDLER_DENSITY: &str = "ui/event_handler_density";
    pub const DUPLICATE_STRUCTURE: &str = "ui/duplicate_structure";
    pub const ARCHITECTURE_CYCLE: &str = "architecture/dependency_cycle";
    pub const ARCHITECTURE_HIGH_FAN_OUT: &str = "architecture/high_fan_out";
    pub const ARCHITECTURE_LAYER_VIOLATION: &str = "architecture/layer_violation";
}

/// 把相对路径规范化为 ID 用字符串：统一 `/` 分隔、去掉前导 `./`。
/// 不要求路径真实存在——差异计算在扫描态使用，路径来自同一项目根下的
/// 相对路径，规范化只是为了跨平台（Windows 反斜杠）与手工构造一致。
pub fn normalize_path_for_id(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('\\', "/");
    while let Some(stripped) = s.strip_prefix("./") {
        s = stripped.to_string();
    }
    s
}

/// 计算稳定发现 ID：对 `rule_id + 规范化路径 + symbol + 结构签名` 做
/// FNV-1a 64 位哈希，输出 16 位十六进制串。不依赖随机源（`DefaultHasher`
/// 的 SipHash 键由 `new()` 固定，但可读性差且版本间行为不够直白；FNV-1a
/// 无依赖、跨运行确定，足够做“稳定身份”而非密码学用途）。
pub fn stable_finding_id(
    rule_id: &str,
    path: &Path,
    symbol: Option<&str>,
    signature: &str,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(rule_id.as_bytes());
    feed(normalize_path_for_id(path).as_bytes());
    feed(symbol.unwrap_or("").as_bytes());
    feed(signature.as_bytes());
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_ignores_line_numbers_and_metric_values() {
        let a = stable_finding_id(
            "structure/complexity_signal",
            Path::new("src/a.rs"),
            Some("foo"),
            "foo",
        );
        let b = stable_finding_id(
            "structure/complexity_signal",
            Path::new("src/a.rs"),
            Some("foo"),
            "foo",
        );
        assert_eq!(a, b);
        // 行号/指标值不参与计算，故无论传什么都得到相同结果（ID 由签名而非
        // 具体数值决定，测试只验证确定性与组成字段的隔离性）。
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn stable_id_changes_when_signature_differs() {
        let a = stable_finding_id(
            "structure/complexity_signal",
            Path::new("a.rs"),
            Some("foo"),
            "foo",
        );
        let b = stable_finding_id(
            "structure/complexity_signal",
            Path::new("a.rs"),
            Some("bar"),
            "bar",
        );
        assert_ne!(a, b);
    }

    #[test]
    fn stable_id_changes_when_rule_differs() {
        let a = stable_finding_id("ui/color_hardcode", Path::new("a.rs"), None, "x");
        let b = stable_finding_id("ui/spacing_hardcode", Path::new("a.rs"), None, "x");
        assert_ne!(a, b);
    }

    #[test]
    fn normalize_path_strips_dot_slash_and_unifies_separators() {
        assert_eq!(normalize_path_for_id(Path::new("./src/a.rs")), "src/a.rs");
        assert_eq!(normalize_path_for_id(Path::new("src\\a.rs")), "src/a.rs");
    }

    #[test]
    fn evidence_metric_value_matches_kind() {
        assert_eq!(
            FindingEvidence::Structure {
                complexity_signal: 41,
                loc: 5,
                widget_nesting_depth: 0,
                event_handler_count: 0,
            }
            .metric_value(),
            Some(41)
        );
        assert_eq!(
            FindingEvidence::Duplicate { occurrences: 7 }.metric_value(),
            Some(7)
        );
        assert_eq!(
            FindingEvidence::NestingDepth { depth: 5 }.metric_value(),
            Some(5)
        );
        assert_eq!(
            FindingEvidence::EventHandlers { count: 8 }.metric_value(),
            Some(8)
        );
        assert_eq!(
            FindingEvidence::Literal {
                snippet: "16".into()
            }
            .metric_value(),
            None
        );
    }

    #[test]
    fn architecture_hub_metric_is_fan_out() {
        assert_eq!(
            FindingEvidence::ArchitectureHub {
                node_id: "module:a".into(),
                fan_out: 12,
            }
            .metric_value(),
            Some(12)
        );
    }

    #[test]
    fn architecture_cycle_and_boundary_have_no_metric() {
        assert_eq!(
            FindingEvidence::ArchitectureCycle {
                node_ids: vec!["module:a".into(), "module:b".into()],
            }
            .metric_value(),
            None
        );
        assert_eq!(
            FindingEvidence::ArchitectureBoundary {
                edge_id: "edge:module_use:module:a:module:b".into(),
                from_layer: "ui".into(),
                to_layer: "domain".into(),
            }
            .metric_value(),
            None
        );
    }

    #[test]
    fn architecture_category_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FindingCategory::Architecture).unwrap(),
            "\"architecture\""
        );
    }
}
