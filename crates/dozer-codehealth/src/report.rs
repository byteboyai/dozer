use crate::finding::{
    Applicability, Finding, FindingCategory, FindingEvidence, FindingSeverity, rule_ids,
    stable_finding_id,
};
use crate::function_metric::{FunctionMetric, Severity};
use crate::scan_metadata::{GitSnapshot, SCHEMA_VERSION, ScanMetadata, ScanStatus};
use ast_grep_language::{LanguageExt, SupportLang};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `schema_version` 缺省值：旧 JSON 没有该字段，迁移为 v1 报告。
fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthTier {
    Healthy,
    Watch,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileMetric {
    pub path: PathBuf,
    pub loc: usize,
    pub critical_functions: usize,
    pub watch_functions: usize,
    pub flagged: bool,
}

/// spec「文件级」：含 ≥1 个 Critical 函数，或文件总行数 > 1000 → flagged。
pub fn file_metric(path: &Path, file_loc: usize, functions: &[FunctionMetric]) -> FileMetric {
    let critical_functions = functions
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let watch_functions = functions
        .iter()
        .filter(|f| f.severity == Severity::Watch)
        .count();
    FileMetric {
        path: path.to_path_buf(),
        loc: file_loc,
        critical_functions,
        watch_functions,
        flagged: critical_functions > 0 || file_loc > 1000,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReport {
    /// 报告 schema 版本。v2 起才有版本化；旧 JSON 缺字段回落 1。
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 扫描范围元数据。旧 JSON 缺字段时回落全零默认值。
    #[serde(default)]
    pub scan: ScanMetadata,
    /// 扫描时的 Git 基准。`dozer-codehealth` 是纯分析库、不自采，由应用层
    /// 扫描前采集填入；旧 JSON / 库内直接扫描时为 `None`。
    #[serde(default)]
    pub git: Option<GitSnapshot>,
    /// 统一发现项列表（由下述各 legacy 指标转换而来，同一次扫描结果）。
    /// 旧 JSON 缺字段时回落空列表。
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// 架构图（schema v3）。旧 JSON 缺字段时回落“不适用”空报告，绝不能
    /// 被解读为“架构健康”（spec「旧报告不得被解读为健康」）。
    #[serde(default)]
    pub architecture: crate::architecture::ArchitectureReport,

    // —— 以下为 legacy 字段，迁移期保留，后续由 findings 聚合替代 ——
    pub total_loc: usize,
    pub total_functions: usize,
    pub critical_functions: usize,
    pub scale_tier: HealthTier,
    pub density_tier: HealthTier,
    pub overall_tier: HealthTier,
    pub functions: Vec<FunctionMetric>,

    pub color_findings: Vec<crate::ui_metrics::RawLiteralFinding>,
    pub color_tier: HealthTier,
    pub spacing_findings: Vec<crate::ui_metrics::RawLiteralFinding>,
    pub spacing_tier: HealthTier,
    pub font_findings: Vec<crate::ui_metrics::RawLiteralFinding>,
    pub font_tier: HealthTier,
    pub distinct_color_values: usize,
    pub distinct_spacing_values: usize,
    pub duplicate_clusters: Vec<crate::ui_metrics::DuplicateCluster>,
    pub duplicate_cluster_tier: HealthTier,
    pub nesting_depth_tier: HealthTier,
    pub event_handler_tier: HealthTier,
    pub ui_tier: HealthTier,
}

/// spec「规模分档」：`< 10_000` Healthy，`10_000..=20_000` Watch，`> 20_000` Critical。
pub fn scale_tier(total_loc: usize) -> HealthTier {
    if total_loc > 20_000 {
        HealthTier::Critical
    } else if total_loc >= 10_000 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「密度分档」：`< 2%` Healthy，`2%..=5%` Watch，`> 5%` Critical。
/// `total_functions == 0` 视为 Healthy（没有函数就没有问题函数可言）。
pub fn density_tier(critical_functions: usize, total_functions: usize) -> HealthTier {
    if total_functions == 0 {
        return HealthTier::Healthy;
    }
    let pct = critical_functions as f64 / total_functions as f64 * 100.0;
    if pct > 5.0 {
        HealthTier::Critical
    } else if pct >= 2.0 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// `Severity`（函数级，含 Normal）→ `FindingSeverity`（只保留 Watch/Critical）。
/// 只有真正构成问题的函数才产出发现项；Normal 不产出。
fn severity_to_finding(s: Severity) -> FindingSeverity {
    match s {
        Severity::Critical => FindingSeverity::Critical,
        Severity::Normal | Severity::Watch => FindingSeverity::Watch,
    }
}

/// `HealthTier`（维度级，含 Healthy）→ `FindingSeverity`。字面量发现共享其
/// 所属维度的档位：维度 Healthy 时本就不产出发现项（数量为 0），此处
/// `Healthy` 被折叠成 Watch 只作兜底，调用方保证不传 Healthy。
fn tier_to_finding_severity(t: HealthTier) -> FindingSeverity {
    match t {
        HealthTier::Critical => FindingSeverity::Critical,
        HealthTier::Healthy | HealthTier::Watch => FindingSeverity::Watch,
    }
}

/// 把一次扫描收集到的全部 legacy 指标转换成统一 [`Finding`] 列表。旧字段与
/// 统一发现项来自同一次扫描结果（spec「统一发现项」）。结构复杂度/嵌套/
/// 回调按“函数”粒度产出，字面量按“单个硬编码处”粒度产出，重复结构按
/// “簇”粒度产出。
///
/// 参数多且有多个同类型相邻（三组 `&[RawLiteralFinding]` + 三个 `HealthTier`），
/// 用具名字段的结构体承载，避免顺序传错编译器查不出（Rust 设计模式）。
struct FindingInputs<'a> {
    functions: &'a [FunctionMetric],
    color_findings: &'a [crate::ui_metrics::RawLiteralFinding],
    color_tier: HealthTier,
    spacing_findings: &'a [crate::ui_metrics::RawLiteralFinding],
    spacing_tier: HealthTier,
    font_findings: &'a [crate::ui_metrics::RawLiteralFinding],
    font_tier: HealthTier,
    duplicate_clusters: &'a [crate::ui_metrics::DuplicateCluster],
}

fn build_findings(inputs: &FindingInputs<'_>) -> Vec<Finding> {
    use crate::ui_metrics::{event_handler_tier, nesting_depth_tier};

    let FindingInputs {
        functions,
        color_findings,
        color_tier,
        spacing_findings,
        spacing_tier,
        font_findings,
        font_tier,
        duplicate_clusters,
    } = *inputs;

    let mut out = Vec::new();

    // 结构复杂度：控制流信号（Watch/Critical 函数各一条）。
    for f in functions.iter().filter(|f| f.severity != Severity::Normal) {
        let id = stable_finding_id(
            rule_ids::STRUCTURE_COMPLEXITY,
            &f.file,
            Some(&f.name),
            &f.identity,
        );
        out.push(Finding {
            id,
            rule_id: rule_ids::STRUCTURE_COMPLEXITY.to_string(),
            category: FindingCategory::Structure,
            severity: severity_to_finding(f.severity),
            path: f.file.clone(),
            start_line: f.start_line,
            symbol: Some(f.name.clone()),
            title: format!("{} 控制流信号 {}", f.name, f.complexity_signal),
            evidence: FindingEvidence::Structure {
                complexity_signal: f.complexity_signal,
                loc: f.loc,
                widget_nesting_depth: f.widget_nesting_depth,
                event_handler_count: f.event_handler_count,
            },
            applicability: Applicability::Applicable,
        });
    }

    // 字面量硬编码（颜色/边距/字体）：每个字面量一条。
    for (findings, rule, tier) in [
        (color_findings, rule_ids::COLOR_HARDCODE, color_tier),
        (spacing_findings, rule_ids::SPACING_HARDCODE, spacing_tier),
        (font_findings, rule_ids::FONT_HARDCODE, font_tier),
    ] {
        let severity = tier_to_finding_severity(tier);
        let mut occurrences = std::collections::HashMap::<(&Path, &str), usize>::new();
        for lit in findings {
            let ordinal = occurrences
                .entry((lit.file.as_path(), lit.snippet.as_str()))
                .and_modify(|n| *n += 1)
                .or_insert(0);
            let signature = format!("{}#{ordinal}", lit.snippet);
            let id = stable_finding_id(rule, &lit.file, None, &signature);
            out.push(Finding {
                id,
                rule_id: rule.to_string(),
                category: FindingCategory::UiConsistency,
                severity,
                path: lit.file.clone(),
                start_line: lit.line,
                symbol: None,
                title: literal_title(rule),
                evidence: FindingEvidence::Literal {
                    snippet: lit.snippet.clone(),
                },
                applicability: Applicability::Applicable,
            });
        }
    }

    // 组件树嵌套深度：每个超标函数一条。
    for f in functions
        .iter()
        .filter(|f| nesting_depth_tier(f.widget_nesting_depth) != HealthTier::Healthy)
    {
        let severity = tier_to_finding_severity(nesting_depth_tier(f.widget_nesting_depth));
        let id = stable_finding_id(rule_ids::NESTING_DEPTH, &f.file, Some(&f.name), &f.identity);
        out.push(Finding {
            id,
            rule_id: rule_ids::NESTING_DEPTH.to_string(),
            category: FindingCategory::UiConsistency,
            severity,
            path: f.file.clone(),
            start_line: f.start_line,
            symbol: Some(f.name.clone()),
            title: format!("{} 组件嵌套深度 {}", f.name, f.widget_nesting_depth),
            evidence: FindingEvidence::NestingDepth {
                depth: f.widget_nesting_depth,
            },
            applicability: Applicability::Applicable,
        });
    }

    // 事件回调密度：每个超标函数一条。
    for f in functions
        .iter()
        .filter(|f| event_handler_tier(f.event_handler_count) != HealthTier::Healthy)
    {
        let severity = tier_to_finding_severity(event_handler_tier(f.event_handler_count));
        let id = stable_finding_id(
            rule_ids::EVENT_HANDLER_DENSITY,
            &f.file,
            Some(&f.name),
            &f.identity,
        );
        out.push(Finding {
            id,
            rule_id: rule_ids::EVENT_HANDLER_DENSITY.to_string(),
            category: FindingCategory::UiConsistency,
            severity,
            path: f.file.clone(),
            start_line: f.start_line,
            symbol: Some(f.name.clone()),
            title: format!("{} 回调数 {}", f.name, f.event_handler_count),
            evidence: FindingEvidence::EventHandlers {
                count: f.event_handler_count,
            },
            applicability: Applicability::Applicable,
        });
    }

    // 组件化重复结构：每个簇一条（以首个出现位置为定位）。
    for cluster in duplicate_clusters {
        let (first_path, first_line) = cluster
            .occurrences
            .first()
            .cloned()
            .unwrap_or_else(|| (PathBuf::new(), 0));
        let id = stable_finding_id(
            rule_ids::DUPLICATE_STRUCTURE,
            &first_path,
            None,
            &cluster.signature,
        );
        out.push(Finding {
            id,
            rule_id: rule_ids::DUPLICATE_STRUCTURE.to_string(),
            category: FindingCategory::UiConsistency,
            severity: tier_to_finding_severity(crate::ui_metrics::cluster_tier(
                cluster.occurrences.len(),
            )),
            path: first_path,
            start_line: first_line,
            symbol: None,
            title: format!("重复结构（出现 {} 次）", cluster.occurrences.len()),
            evidence: FindingEvidence::Duplicate {
                occurrences: cluster.occurrences.len(),
            },
            applicability: Applicability::Applicable,
        });
    }

    out
}

fn literal_title(rule: &str) -> String {
    match rule {
        rule_ids::COLOR_HARDCODE => "颜色硬编码".to_string(),
        rule_ids::SPACING_HARDCODE => "边距硬编码".to_string(),
        rule_ids::FONT_HARDCODE => "字体硬编码".to_string(),
        _ => "字面量硬编码".to_string(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport> {
    use crate::ui_metrics::{
        self, Patterns, clusters_from_registry, color_tier, event_handler_tier, font_tier,
        nesting_depth_tier, spacing_tier,
    };
    use std::collections::HashMap;

    let started = now_ms();

    // 非目录（项目路径失效）→ 明确失败状态，不冒充“尚未扫描”。
    if !root.is_dir() {
        return Ok(ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                started_at_ms: started,
                duration_ms: now_ms().saturating_sub(started),
                status: ScanStatus::Failed,
                ..ScanMetadata::default()
            },
            git: None,
            findings: Vec::new(),
            architecture: crate::architecture::ArchitectureReport::not_applicable(),
            total_loc: 0,
            total_functions: 0,
            critical_functions: 0,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Healthy,
            overall_tier: HealthTier::Healthy,
            functions: Vec::new(),
            color_findings: Vec::new(),
            color_tier: HealthTier::Healthy,
            spacing_findings: Vec::new(),
            spacing_tier: HealthTier::Healthy,
            font_findings: Vec::new(),
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: Vec::new(),
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        });
    }

    let config = crate::discovery::load_project_config(root);
    let disc = crate::discovery::discover(root, &config);
    let mut skipped = disc.skipped;

    let patterns = Patterns::compile(SupportLang::Rust);

    let mut all_functions = Vec::new();
    let mut total_loc = 0usize;
    let mut analyzed_files = 0usize;
    let mut color_findings = Vec::new();
    let mut spacing_findings = Vec::new();
    let mut font_findings = Vec::new();
    let mut duplicate_registry: HashMap<String, Vec<(PathBuf, usize)>> = HashMap::new();
    let mut iced_detected = false;
    // module 图：复用同一次解析的 AST 收集 use，不再二次解析（spec 性能约束）。
    let mut module_files: Vec<(
        crate::module_architecture::FileModule,
        Vec<crate::module_architecture::UsePath>,
    )> = Vec::new();
    let mut module_declarations = std::collections::BTreeMap::new();

    for rel in &disc.rust_files {
        let abs = root.join(rel);
        let bytes = match std::fs::read(&abs) {
            Ok(b) => b,
            Err(_) => {
                skipped.push(crate::scan_metadata::SkippedFile {
                    path: rel.clone(),
                    reason: crate::scan_metadata::SkipReason::ReadFailed,
                });
                continue;
            }
        };
        let src = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                skipped.push(crate::scan_metadata::SkippedFile {
                    path: rel.clone(),
                    reason: crate::scan_metadata::SkipReason::NonUtf8,
                });
                continue;
            }
        };

        let ast = SupportLang::Rust.ast_grep(&src);
        let root_node = ast.root();
        // tree-sitter 出错恢复仍产出带 ERROR 节点的树；有 ERROR 节点即视为
        // “解析失败”，整文件跳过并记录原因（spec「错误处理」）。
        if root_node.dfs().any(|n| n.is_error()) {
            skipped.push(crate::scan_metadata::SkippedFile {
                path: rel.clone(),
                reason: crate::scan_metadata::SkipReason::ParseFailed,
            });
            continue;
        }

        analyzed_files += 1;
        total_loc += src.lines().count();
        iced_detected |=
            src.contains("iced_widget") || src.contains("use iced::") || src.contains("iced::");
        // 同一棵 AST 同时喂结构复杂度与 UI 规则，避免重复解析（spec 性能约束）。
        all_functions.extend(crate::function_metric::functions_in_source(
            &root_node, rel, &patterns,
        ));
        color_findings.extend(ui_metrics::find_color_findings(&root_node, &patterns, rel));
        spacing_findings.extend(ui_metrics::find_spacing_findings(
            &root_node, &patterns, rel,
        ));
        font_findings.extend(ui_metrics::find_font_findings(&root_node, &patterns, rel));
        ui_metrics::find_duplicate_clusters(&root_node, rel, &mut duplicate_registry);

        // 同一次解析顺带提取 use（module 图），避免二次 parse。
        module_files.push((
            crate::module_architecture::FileModule { path: rel.clone() },
            crate::module_architecture::uses_in_file(&root_node),
        ));
        module_declarations.insert(
            rel.clone(),
            crate::module_architecture::module_declarations_in_file(&root_node),
        );
    }

    let total_functions = all_functions.len();
    let critical_functions = all_functions
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let scale = scale_tier(total_loc);
    let density = density_tier(critical_functions, total_functions);
    let mut functions = all_functions;
    functions.sort_by_key(|f| std::cmp::Reverse(f.severity_rank()));

    let distinct_color_values: usize = color_findings
        .iter()
        .map(|f| f.snippet.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let distinct_spacing_values: usize = spacing_findings
        .iter()
        .map(|f| f.snippet.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    let duplicate_clusters = clusters_from_registry(duplicate_registry);
    let color_t = color_tier(color_findings.len());
    let spacing_t = spacing_tier(spacing_findings.len());
    let font_t = font_tier(font_findings.len());
    let duplicate_t = duplicate_clusters
        .first()
        .map(|c| crate::ui_metrics::cluster_tier(c.occurrences.len()))
        .unwrap_or(HealthTier::Healthy);
    let nesting_t = functions
        .iter()
        .map(|f| nesting_depth_tier(f.widget_nesting_depth))
        .max()
        .unwrap_or(HealthTier::Healthy);
    let handler_t = functions
        .iter()
        .map(|f| event_handler_tier(f.event_handler_count))
        .max()
        .unwrap_or(HealthTier::Healthy);
    let ui_tier = [
        color_t,
        spacing_t,
        font_t,
        duplicate_t,
        nesting_t,
        handler_t,
    ]
    .into_iter()
    .max()
    .unwrap();

    let mut findings = build_findings(&FindingInputs {
        functions: &functions,
        color_findings: &color_findings,
        color_tier: color_t,
        spacing_findings: &spacing_findings,
        spacing_tier: spacing_t,
        font_findings: &font_findings,
        font_tier: font_t,
        duplicate_clusters: &duplicate_clusters,
    });
    if !iced_detected {
        for finding in findings
            .iter_mut()
            .filter(|f| f.category == FindingCategory::UiConsistency)
        {
            finding.applicability = Applicability::NotApplicable;
        }
    }

    // 架构图：Cargo workspace + module 图（module 复用上面的 AST，见扫描循环）。
    // 提取失败可降级，不影响结构复杂度等其它结果。
    let cargo = crate::cargo_architecture::extract_cargo_architecture(root);
    let (cargo_status, mut cargo_nodes, cargo_edges, arch_error) =
        crate::cargo_architecture::cargo_report_fragment(&cargo);

    // module 图：crate → src 目录来自 Cargo；workspace crate 名集合用于解析跨
    // crate use。
    let mut workspace_crates = std::collections::BTreeSet::new();
    let mut crate_roots = Vec::new();
    if let crate::cargo_architecture::CargoArchitecture::Ok(graph) = &cargo {
        workspace_crates.extend(graph.member_roots.iter().map(|r| r.crate_name.clone()));
        crate_roots.extend(graph.member_roots.clone());
    } else if !module_files.is_empty() {
        // 非 Cargo Rust 项目也提供 module 图：选 lib.rs/main.rs；都没有时用首个
        // Rust 文件作为 synthetic crate root，并明确保持 Partial。
        let project_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("project")
            .to_string();
        let paths: std::collections::BTreeSet<_> =
            module_files.iter().map(|(f, _)| f.path.clone()).collect();
        let roots: Vec<_> = ["lib.rs", "main.rs"]
            .into_iter()
            .map(std::path::PathBuf::from)
            .filter(|p| paths.contains(p))
            .collect();
        let roots = if roots.is_empty() {
            paths.iter().next().cloned().into_iter().collect()
        } else {
            roots
        };
        workspace_crates.insert(project_name.clone());
        crate_roots.push(crate::module_architecture::CrateRoots {
            crate_name: project_name,
            targets: roots
                .into_iter()
                .map(|root_file| crate::module_architecture::TargetRoot {
                    source_dir: root_file
                        .parent()
                        .unwrap_or(std::path::Path::new(""))
                        .to_path_buf(),
                    root_file,
                    module_prefix: Vec::new(),
                })
                .collect(),
            dependency_aliases: std::collections::BTreeMap::new(),
        });
    }
    let module_graph = crate::module_architecture::build_module_graph_with_declarations(
        &module_files,
        &module_declarations,
        &crate_roots,
        &workspace_crates,
    );
    for roots in &crate_roots {
        let id = crate::architecture::crate_node_id(&roots.crate_name);
        if !cargo_nodes.iter().any(|n| n.id == id) {
            cargo_nodes.push(crate::architecture::ArchitectureNode {
                id,
                kind: crate::architecture::ArchitectureNodeKind::Crate,
                name: roots.crate_name.clone(),
                qualified_name: roots.crate_name.clone(),
                path: roots.targets.first().map(|t| t.root_file.clone()),
                parent_id: None,
                loc: 0,
                fan_in: 0,
                fan_out: 0,
                layer: None,
                external: false,
            });
        }
    }

    // 合并节点：按 id 去重（crate 节点来自 Cargo，module 节点来自 module 图）。
    let mut arch_nodes: Vec<crate::architecture::ArchitectureNode> = cargo_nodes;
    {
        let mut seen: std::collections::HashSet<String> =
            arch_nodes.iter().map(|n| n.id.clone()).collect();
        for node in module_graph.nodes {
            if seen.insert(node.id.clone()) {
                arch_nodes.push(node);
            }
        }
    }
    let mut arch_edges = cargo_edges;
    arch_edges.extend(module_graph.edges);
    let arch_edges: Vec<_> = {
        let mut seen = std::collections::HashSet::new();
        arch_edges
            .into_iter()
            .filter(|e| seen.insert(e.id.clone()))
            .collect()
    };

    // 状态：Cargo 缺省 NotApplicable，但有 module 数据时升级为 Partial；失败保持
    // Partial 并记录错误。
    let status_from_cargo = match cargo_status {
        crate::architecture::ArchitectureStatus::NotApplicable => {
            if !arch_nodes.is_empty() || !arch_edges.is_empty() {
                crate::architecture::ArchitectureStatus::Partial
            } else {
                crate::architecture::ArchitectureStatus::NotApplicable
            }
        }
        other => other,
    };

    // 图分析：写回 fan-in/out 与 layer，检测循环与三类架构发现（spec「图分析
    // 与健康规则」）。配置错误可定位记录，并使架构/扫描状态为 Partial。
    let config_errors = crate::discovery::validate_architecture_config(&config.architecture);
    let report_for_analysis = crate::architecture::ArchitectureReport {
        status: status_from_cargo,
        nodes: arch_nodes,
        edges: arch_edges,
        cycles: Vec::new(),
        unresolved_edges: module_graph.unresolved_edges,
        errors: arch_error.into_iter().collect(),
    };
    let analysis = crate::architecture_analysis::analyze_architecture(
        &report_for_analysis,
        &config.architecture,
    );
    let mut arch_errors = report_for_analysis.errors;
    arch_errors.extend(config.config_errors.clone());
    arch_errors.extend(analysis.config_errors);
    arch_errors.extend(config_errors);
    arch_errors.sort();
    arch_errors.dedup();

    let architecture = crate::architecture::ArchitectureReport {
        status: if arch_errors.is_empty() {
            report_for_analysis.status
        } else if report_for_analysis.status
            == crate::architecture::ArchitectureStatus::NotApplicable
        {
            // 仅有配置错误、没有任何图数据时，不应把“无分析”升级为 Partial；
            // 保持 NotApplicable 但保留错误文案（spec 空状态语义）。
            crate::architecture::ArchitectureStatus::NotApplicable
        } else {
            crate::architecture::ArchitectureStatus::Partial
        },
        nodes: analysis.nodes,
        edges: report_for_analysis.edges,
        cycles: analysis.cycles,
        unresolved_edges: report_for_analysis.unresolved_edges,
        errors: arch_errors,
    };

    findings.extend(analysis.findings);

    let status = if skipped.iter().any(|s| {
        matches!(
            s.reason,
            crate::scan_metadata::SkipReason::NonUtf8
                | crate::scan_metadata::SkipReason::ReadFailed
                | crate::scan_metadata::SkipReason::ParseFailed
        )
    }) || !architecture.errors.is_empty()
    {
        ScanStatus::Partial
    } else {
        ScanStatus::Complete
    };

    let scan = ScanMetadata {
        started_at_ms: started,
        duration_ms: now_ms().saturating_sub(started),
        status,
        discovered_files: disc.discovered_count,
        analyzed_files,
        excluded_files: disc.excluded_count,
        skipped_files: skipped,
        languages: disc.languages,
        frameworks: if iced_detected {
            vec!["iced".to_string()]
        } else {
            Vec::new()
        },
    };

    Ok(ProjectReport {
        schema_version: SCHEMA_VERSION,
        scan,
        git: None,
        findings,
        architecture,
        total_loc,
        total_functions,
        critical_functions,
        scale_tier: scale,
        density_tier: density,
        overall_tier: scale.max(density),
        functions,
        color_findings,
        color_tier: color_t,
        spacing_findings,
        spacing_tier: spacing_t,
        font_findings,
        font_tier: font_t,
        distinct_color_values,
        distinct_spacing_values,
        duplicate_clusters,
        duplicate_cluster_tier: duplicate_t,
        nesting_depth_tier: nesting_t,
        event_handler_tier: handler_t,
        ui_tier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function_metric::severity_for;

    #[test]
    fn health_tier_ordered_critical_highest() {
        assert!(HealthTier::Critical > HealthTier::Watch);
        assert!(HealthTier::Watch > HealthTier::Healthy);
        assert_eq!(
            HealthTier::Healthy.max(HealthTier::Critical),
            HealthTier::Critical
        );
    }

    #[test]
    fn file_metric_flags_on_critical_function() {
        let f = FunctionMetric {
            name: "x".into(),
            identity: "x".into(),
            file: PathBuf::from("a.rs"),
            start_line: 1,
            end_line: 2,
            loc: 2,
            complexity_signal: 41,
            severity: severity_for(41),
            widget_nesting_depth: 0,
            event_handler_count: 0,
        };
        let m = file_metric(Path::new("a.rs"), 100, &[f]);
        assert_eq!(m.critical_functions, 1);
        assert_eq!(m.watch_functions, 0);
        assert!(m.flagged);
    }

    #[test]
    fn file_metric_flags_on_loc_over_1000_even_without_critical_function() {
        let m = file_metric(Path::new("a.rs"), 1001, &[]);
        assert!(m.flagged);
    }

    #[test]
    fn file_metric_not_flagged_when_small_and_no_critical() {
        let m = file_metric(Path::new("a.rs"), 1000, &[]);
        assert!(!m.flagged);
    }

    #[test]
    fn scale_tier_boundaries() {
        assert_eq!(scale_tier(9_999), HealthTier::Healthy);
        assert_eq!(scale_tier(10_000), HealthTier::Watch);
        assert_eq!(scale_tier(20_000), HealthTier::Watch);
        assert_eq!(scale_tier(20_001), HealthTier::Critical);
    }

    #[test]
    fn density_tier_boundaries() {
        assert_eq!(density_tier(1, 100), HealthTier::Healthy); // 1%
        assert_eq!(density_tier(2, 100), HealthTier::Watch); // 2%
        assert_eq!(density_tier(5, 100), HealthTier::Watch); // 5%
        assert_eq!(density_tier(6, 100), HealthTier::Critical); // 6%
    }

    #[test]
    fn density_tier_zero_functions_is_healthy() {
        assert_eq!(density_tier(0, 0), HealthTier::Healthy);
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_project_empty_dir_returns_all_zero() {
        let dir = tempfile::tempdir().unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_loc, 0);
        assert_eq!(report.total_functions, 0);
        assert_eq!(report.critical_functions, 0);
        assert_eq!(report.overall_tier, HealthTier::Healthy);
        assert!(report.functions.is_empty());
    }

    #[test]
    fn scan_project_skips_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/generated.rs"), "fn ignored() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 0);
    }

    #[test]
    fn scan_project_skips_unparseable_file_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("good.rs"), "fn ok() {}").unwrap();
        // 非法 UTF-8 字节序列——`fs::read_to_string` 会报错，该文件应被跳过
        // 而不是让整体扫描失败。
        fs::write(dir.path().join("bad.rs"), [0xff, 0xfe, 0x00]).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 1);
    }

    #[test]
    fn scan_project_aggregates_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        fs::write(dir.path().join("c.rs"), "fn c() {}\n").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 3);
    }

    #[test]
    fn scan_project_overall_tier_takes_worse_of_scale_and_density() {
        let dir = tempfile::tempdir().unwrap();
        // 造一个复杂度信号 41(Critical)的函数,规模很小(远低于 1 万行),
        // scale_tier 应为 Healthy、density_tier 应为 Critical(1/1 = 100%),
        // overall_tier 取二者较严重者 = Critical。
        let body = "if a {}".repeat(41 - 1); // 40 个 if 之外再手写一个,凑够 41
        let src = format!("fn heavy() {{\n    if a {{}}\n{body}\n}}\n");
        fs::write(dir.path().join("heavy.rs"), src).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.scale_tier, HealthTier::Healthy);
        assert_eq!(report.density_tier, HealthTier::Critical);
        assert_eq!(report.overall_tier, HealthTier::Critical);
    }

    #[test]
    fn scan_project_collects_color_findings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.rs"),
            "fn f() { let c = Color::from_rgb(0.1, 0.2, 0.3); }",
        )
        .unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.color_findings.len(), 1);
        assert_eq!(report.color_tier, HealthTier::Watch);
    }

    #[test]
    fn scan_project_computes_distinct_spacing_values() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.rs"),
            "fn f() { col.padding(8).padding(8).spacing(16); }",
        )
        .unwrap();
        let report = scan_project(dir.path()).unwrap();
        // 8 出现两次、16 一次,distinct 应该是 2。
        assert_eq!(report.distinct_spacing_values, 2);
    }

    #[test]
    fn scan_project_finds_duplicate_clusters_across_files() {
        let dir = tempfile::tempdir().unwrap();
        for (i, body) in ["x", "y", "z"].iter().enumerate() {
            fs::write(
                dir.path().join(format!("f{i}.rs")),
                format!("fn f() {{ row![text(\"{body}\")] }}"),
            )
            .unwrap();
        }
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.duplicate_clusters.len(), 1);
        assert_eq!(report.duplicate_clusters[0].occurrences.len(), 3);
    }

    #[test]
    fn scan_project_ui_tier_takes_worst_of_six_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        // 16 处颜色硬编码 -> color_tier = Critical,其余维度都是 Healthy,
        // ui_tier 应该跟着变成 Critical。
        let calls: String = (0..16)
            .map(|i| format!("let _c{i} = Color::from_rgb(0.1, 0.2, 0.3);\n"))
            .collect();
        fs::write(dir.path().join("a.rs"), format!("fn f() {{\n{calls}}}\n")).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.color_tier, HealthTier::Critical);
        assert_eq!(report.ui_tier, HealthTier::Critical);
    }

    #[test]
    fn scan_project_empty_dir_ui_fields_all_zero() {
        let dir = tempfile::tempdir().unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert!(report.color_findings.is_empty());
        assert!(report.spacing_findings.is_empty());
        assert!(report.font_findings.is_empty());
        assert!(report.duplicate_clusters.is_empty());
        assert_eq!(report.distinct_color_values, 0);
        assert_eq!(report.distinct_spacing_values, 0);
        assert_eq!(report.ui_tier, HealthTier::Healthy);
    }
}

#[cfg(test)]
mod v2_tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_project_reports_current_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn ok() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert_eq!(report.schema_version, 3);
        assert_eq!(report.scan.status, ScanStatus::Complete);
    }

    #[test]
    fn scan_without_cargo_manifest_builds_partial_synthetic_graph() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn ok() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(
            report.architecture.status,
            crate::architecture::ArchitectureStatus::Partial
        );
        assert!(report.architecture.has_data());
        assert!(report.architecture.errors.is_empty());
        assert_eq!(report.scan.status, ScanStatus::Complete);
    }

    #[test]
    fn scan_with_cargo_workspace_populates_architecture() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(root.join("a/src/lib.rs"), "pub fn f() {}\n").unwrap();
        let report = scan_project(root).unwrap();
        assert_eq!(
            report.architecture.status,
            crate::architecture::ArchitectureStatus::Complete
        );
        assert!(
            report
                .architecture
                .nodes
                .iter()
                .any(|n| n.id == crate::architecture::crate_node_id("a"))
        );
    }

    #[test]
    fn scan_populates_module_graph_from_shared_ast() {
        // 一个 workspace member 里有子 module，lib.rs `use crate::foo::Bar;`：
        // 扫描应产出 crate→module 的 ModuleUse 边，且 module 节点挂在 crate 下。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            root.join("a/src/lib.rs"),
            "mod foo;\nuse crate::foo::Bar;\npub fn f() {}\n",
        )
        .unwrap();
        fs::write(root.join("a/src/foo.rs"), "pub struct Bar;\n").unwrap();

        let report = scan_project(root).unwrap();
        let arch = &report.architecture;
        let foo_id = crate::architecture::module_node_id("a", "foo");
        assert!(
            arch.nodes.iter().any(|n| n.id == foo_id),
            "module 节点应存在：{:?}",
            arch.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        let edge = arch
            .edges
            .iter()
            .find(|e| {
                e.kind == crate::architecture::ArchitectureEdgeKind::ModuleUse
                    && e.from == crate::architecture::module_node_id("a", "")
                    && e.to == foo_id
            })
            .expect("crate → foo ModuleUse 边");
        assert_eq!(edge.evidence[0].line, 2);
        assert!(edge.evidence[0].snippet.contains("crate::foo::Bar"));
        assert_eq!(arch.unresolved_edges, 0);
    }

    #[test]
    fn scan_uses_custom_cargo_target_root_and_ignores_orphan_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='custom-root'\nversion='0.1.0'\nedition='2021'\n[lib]\npath='source/root.rs'\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("source")).unwrap();
        fs::write(root.join("source/root.rs"), "mod child;\n").unwrap();
        fs::write(root.join("source/child.rs"), "pub struct Child;\n").unwrap();
        fs::write(root.join("source/orphan.rs"), "pub struct Orphan;\n").unwrap();
        let report = scan_project(root).unwrap();
        assert!(
            report
                .architecture
                .nodes
                .iter()
                .any(|n| n.id == crate::architecture::module_node_id("custom-root", "child"))
        );
        assert!(
            !report
                .architecture
                .nodes
                .iter()
                .any(|n| n.id == crate::architecture::module_node_id("custom-root", "orphan"))
        );
    }

    #[test]
    fn scan_detects_module_cycle_and_emits_finding() {
        // a::x 与 a::y 互相 use → 一个 2 节点 SCC，产生一条 architecture 发现。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(root.join("a/src/lib.rs"), "mod x;\nmod y;\n").unwrap();
        fs::write(root.join("a/src/x.rs"), "use crate::y::Y;\npub struct X;\n").unwrap();
        fs::write(root.join("a/src/y.rs"), "use crate::x::X;\npub struct Y;\n").unwrap();

        let report = scan_project(root).unwrap();
        assert_eq!(report.architecture.cycles.len(), 1, "应检测到 1 个环");
        let cycle = &report.architecture.cycles[0];
        assert_eq!(
            cycle.node_ids,
            vec![
                crate::architecture::module_node_id("a", "x"),
                crate::architecture::module_node_id("a", "y"),
            ]
        );
        let cycle_findings = report
            .findings
            .iter()
            .filter(|f| f.rule_id == rule_ids::ARCHITECTURE_CYCLE)
            .count();
        assert_eq!(cycle_findings, 1);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.category == FindingCategory::Architecture)
        );
    }

    #[test]
    fn invalid_architecture_config_surfaces_errors_as_partial() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(root.join("a/src/lib.rs"), "pub fn f() {}\n").unwrap();
        fs::create_dir_all(root.join(".dozer")).unwrap();
        fs::write(
            root.join(".dozer/code-health.toml"),
            "[architecture]\nmax_fan_out = 10\ncritical_fan_out = 5\n",
        )
        .unwrap();

        let report = scan_project(root).unwrap();
        assert!(
            report
                .architecture
                .errors
                .iter()
                .any(|e| e.contains("critical_fan_out")),
            "配置错误应可定位：{:?}",
            report.architecture.errors
        );
        assert_eq!(
            report.architecture.status,
            crate::architecture::ArchitectureStatus::Partial
        );
        assert_eq!(report.scan.status, ScanStatus::Partial);
    }

    #[test]
    fn scan_without_cargo_manifest_builds_module_graph() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("lib.rs"), "mod foo;\n").unwrap();
        fs::create_dir_all(root.join("foo")).unwrap();
        fs::write(root.join("foo/mod.rs"), "pub struct Bar;\n").unwrap();
        let report = scan_project(root).unwrap();
        assert!(report.architecture.has_data());
        assert_eq!(
            report.architecture.status,
            crate::architecture::ArchitectureStatus::Partial
        );
        assert!(
            report
                .architecture
                .nodes
                .iter()
                .any(|n| n.id.contains("::foo"))
        );
    }

    #[test]
    fn broken_cargo_manifest_degrades_to_partial_with_error() {
        // 有 Cargo.toml 但内容非法 → cargo metadata 失败：架构状态 Partial、
        // 记录错误，其它分析（结构复杂度）仍照常产出，不整次扫描失败。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.toml"), "this is not toml = = =\n").unwrap();
        fs::write(root.join("a.rs"), "fn ok() {}\n").unwrap();
        let report = scan_project(root).unwrap();
        assert_eq!(
            report.architecture.status,
            crate::architecture::ArchitectureStatus::Partial
        );
        assert!(!report.architecture.errors.is_empty());
        assert_eq!(report.scan.status, ScanStatus::Partial);
        assert_eq!(report.total_functions, 1, "其它分析不受影响");
    }

    #[test]
    fn old_v1_json_without_new_fields_deserializes_with_defaults() {
        // 旧版报告 JSON 没有 schema_version/scan/git/findings 四个字段；
        // 反序列化必须靠 serde 默认值迁移为 v1 报告，而不是报缺字段。
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn ok() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        let mut json: serde_json::Value = serde_json::to_value(&report).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("schema_version");
        obj.remove("scan");
        obj.remove("git");
        obj.remove("findings");
        let old_json = serde_json::to_string(&json).unwrap();
        let back: ProjectReport = serde_json::from_str(&old_json).unwrap();
        assert_eq!(back.schema_version, 1);
        assert!(back.findings.is_empty());
        assert_eq!(back.scan.status, ScanStatus::Complete);
        assert!(back.git.is_none());
    }

    #[test]
    fn v2_report_round_trips_with_all_finding_kinds() {
        let dir = tempfile::tempdir().unwrap();
        // 结构复杂度(控制流信号 > 40 → Critical)、颜色硬编码、重复结构三种。
        let heavy = "fn heavy() {\n".to_string() + &"    if a {}\n".repeat(41) + "}\n";
        fs::write(dir.path().join("heavy.rs"), &heavy).unwrap();
        fs::write(
            dir.path().join("color.rs"),
            "fn f() { let c = Color::from_rgb(0.1, 0.2, 0.3); }",
        )
        .unwrap();
        for i in 0..3 {
            fs::write(
                dir.path().join(format!("dup{i}.rs")),
                format!("fn f() {{ row![text(\"{i}\")] }}"),
            )
            .unwrap();
        }
        let report = scan_project(dir.path()).unwrap();
        assert!(!report.findings.is_empty());
        let json = serde_json::to_string(&report).unwrap();
        let back: ProjectReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert!(back.findings.iter().all(|f| !f.id.is_empty()));
    }

    #[test]
    fn v2_json_without_architecture_reads_as_not_applicable() {
        // schema v2 报告没有架构字段；读取后必须回落“不适用”空报告，
        // 且 schema_version 保持原样（不假装是 v3、更不能被解读为架构健康）。
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn ok() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        let mut json: serde_json::Value = serde_json::to_value(&report).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("architecture");
        obj.insert("schema_version".into(), serde_json::json!(2));
        let v2_json = serde_json::to_string(&json).unwrap();
        let back: ProjectReport = serde_json::from_str(&v2_json).unwrap();
        assert_eq!(back.schema_version, 2);
        assert_eq!(
            back.architecture.status,
            crate::architecture::ArchitectureStatus::NotApplicable
        );
        assert!(!back.architecture.has_data());
    }

    #[test]
    fn findings_derive_from_same_scan_as_legacy_fields() {
        let dir = tempfile::tempdir().unwrap();
        let heavy = "fn heavy() {\n".to_string() + &"    if a {}\n".repeat(41) + "}\n";
        fs::write(dir.path().join("heavy.rs"), &heavy).unwrap();
        let report = scan_project(dir.path()).unwrap();
        // legacy 字段与统一发现项来自同一次扫描：结构复杂度 Critical 函数
        // 既在 functions 里，也有一条 structure finding。
        let structure_findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.rule_id == rule_ids::STRUCTURE_COMPLEXITY)
            .collect();
        assert_eq!(structure_findings.len(), 1);
        assert_eq!(structure_findings[0].evidence.metric_value(), Some(41));
        assert_eq!(structure_findings[0].severity, FindingSeverity::Critical);
    }

    #[test]
    fn repeated_identical_literals_have_unique_stable_ids() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("ui.rs"),
            "fn view() { a.padding(8); b.padding(8); }",
        )
        .unwrap();
        let first = scan_project(dir.path()).unwrap();
        let second = scan_project(dir.path()).unwrap();
        let ids = |report: &ProjectReport| {
            report
                .findings
                .iter()
                .filter(|f| f.rule_id == rule_ids::SPACING_HARDCODE)
                .map(|f| f.id.clone())
                .collect::<Vec<_>>()
        };
        let first_ids = ids(&first);
        assert_eq!(first_ids.len(), 2);
        assert_ne!(first_ids[0], first_ids[1]);
        assert_eq!(first_ids, ids(&second));
        let diff = crate::diff_reports(&first.findings, &second.findings);
        assert_eq!(diff.new_count(), 0);
    }

    #[test]
    fn iced_applicability_requires_detected_framework() {
        let plain = tempfile::tempdir().unwrap();
        fs::write(plain.path().join("plain.rs"), "fn view() { a.padding(8); }").unwrap();
        let plain_report = scan_project(plain.path()).unwrap();
        assert!(plain_report.scan.frameworks.is_empty());
        assert!(
            plain_report
                .findings
                .iter()
                .filter(|f| f.category == FindingCategory::UiConsistency)
                .all(|f| f.applicability == Applicability::NotApplicable)
        );

        let iced = tempfile::tempdir().unwrap();
        fs::write(
            iced.path().join("iced.rs"),
            "use iced_widget::button; fn view() { a.padding(8); }",
        )
        .unwrap();
        let iced_report = scan_project(iced.path()).unwrap();
        assert_eq!(iced_report.scan.frameworks, vec!["iced"]);
        assert!(
            iced_report
                .findings
                .iter()
                .filter(|f| f.category == FindingCategory::UiConsistency)
                .all(|f| f.applicability == Applicability::Applicable)
        );
    }
}

#[cfg(test)]
mod scan_scope_tests {
    use super::*;
    use crate::scan_metadata::SkipReason;
    use std::fs;

    #[test]
    fn scan_project_marks_non_utf8_file_as_skipped_and_partial() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("good.rs"), "fn ok() {}").unwrap();
        fs::write(dir.path().join("bad.rs"), [0xff, 0xfe, 0x00]).unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 1);
        assert_eq!(report.scan.status, ScanStatus::Partial);
        assert!(
            report
                .scan
                .skipped_files
                .iter()
                .any(|s| s.path == Path::new("bad.rs") && s.reason == SkipReason::NonUtf8)
        );
    }

    #[test]
    fn scan_project_marks_parse_failed_file_as_skipped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("good.rs"), "fn ok() {}").unwrap();
        // 明显语法错误（缺右括号）——tree-sitter 出错恢复会带 ERROR 节点。
        fs::write(dir.path().join("broken.rs"), "fn broken( {").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.total_functions, 1);
        assert_eq!(report.scan.status, ScanStatus::Partial);
        assert!(
            report
                .scan
                .skipped_files
                .iter()
                .any(|s| s.path == Path::new("broken.rs") && s.reason == SkipReason::ParseFailed)
        );
    }

    #[test]
    fn no_supported_code_returns_success_with_clear_scope() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.md"), "# hi").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert_eq!(report.scan.status, ScanStatus::Complete);
        assert_eq!(report.scan.analyzed_files, 0);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn non_directory_root_returns_failed_status() {
        let report = scan_project(Path::new("/nonexistent/dozer/root")).unwrap();
        assert_eq!(report.scan.status, ScanStatus::Failed);
    }

    #[test]
    fn complete_scan_has_empty_skipped_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.scan.status, ScanStatus::Complete);
        assert!(report.scan.skipped_files.is_empty());
        assert_eq!(report.scan.analyzed_files, 1);
        assert_eq!(report.scan.discovered_files, 1);
    }

    #[test]
    fn excluded_files_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        fs::write(dir.path().join("b.txt"), "hello").unwrap();
        let report = scan_project(dir.path()).unwrap();
        assert_eq!(report.scan.excluded_files, 1);
    }
}

#[cfg(test)]
mod perf_tests {
    use super::*;
    use std::fs;

    // 10 万行 fixture：约 200 文件 × 250 函数 × 2 行。生成本身不计时。
    const TARGET_LINES: usize = 100_000;
    const FUNCS_PER_FILE: usize = 250;

    /// release 构建下扫描 10 万行 Rust 的性能基线（spec 目标 ≤ 15s）。仅用
    /// `--release -- --ignored scan_performance_baseline` 手动跑，不做 CI 门禁。
    #[test]
    #[ignore]
    fn scan_performance_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let n_files = TARGET_LINES / (FUNCS_PER_FILE * 2);
        for i in 0..n_files {
            let mut src = String::new();
            for j in 0..FUNCS_PER_FILE {
                src.push_str(&format!(
                    "fn f{i}_{j}(n: i32) -> i32 {{\n    if n > 0 {{ n }} else {{ n }}\n}}\n"
                ));
            }
            fs::write(dir.path().join(format!("f{i}.rs")), src).unwrap();
        }
        let start = std::time::Instant::now();
        let report = scan_project(dir.path()).unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "scan_performance_baseline: {} files, {} functions, {:.2?} elapsed",
            n_files, report.total_functions, elapsed
        );
        assert_eq!(report.scan.status, ScanStatus::Complete);
        assert!(
            elapsed.as_secs_f64() < 15.0,
            "release 扫描超过 spec 15s 目标: {elapsed:?}"
        );
    }
}
