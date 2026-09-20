use crate::function_metric::{FunctionMetric, Severity};
use ast_grep_language::{LanguageExt, SupportLang};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
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

pub fn scan_project(root: &Path) -> anyhow::Result<ProjectReport> {
    use crate::ui_metrics::{
        self, Patterns, clusters_from_registry, color_tier, event_handler_tier, font_tier,
        nesting_depth_tier, spacing_tier,
    };
    use std::collections::HashMap;

    let mut files = Vec::new();
    if root.is_dir() {
        collect_rs_files(root, &mut files)?;
    }

    let patterns = Patterns::compile(SupportLang::Rust);

    let mut all_functions = Vec::new();
    let mut total_loc = 0usize;
    let mut color_findings = Vec::new();
    let mut spacing_findings = Vec::new();
    let mut font_findings = Vec::new();
    let mut duplicate_registry: HashMap<String, Vec<(PathBuf, usize)>> = HashMap::new();

    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            // 单文件读不到/非 UTF-8 → 跳过,不中断整体扫描(见 spec「错误处理」)。
            continue;
        };
        total_loc += src.lines().count();
        let ast = SupportLang::Rust.ast_grep(&src);
        let root_node = ast.root();
        // 这里和 `functions_in_source` 内部各自独立 `ast_grep(&src)` 解析了
        // 一次同一份源码——重复解析、不是共享同一棵树。已知的小效率损耗:
        // tree-sitter 解析本身很快(spike 瓶颈是 pattern 重复编译,不是解析),
        // 全仓库量级下多一次解析仍然在个位数到十几秒可接受范围(Task 8 会
        // 实测确认),不为省这一次解析去改 `functions_in_source` 签名接收
        // 外部传入的 root 节点,增加不必要的复杂度。

        all_functions.extend(crate::function_metric::functions_in_source(
            &src, path, &patterns,
        ));
        color_findings.extend(ui_metrics::find_color_findings(&root_node, &patterns, path));
        spacing_findings.extend(ui_metrics::find_spacing_findings(
            &root_node, &patterns, path,
        ));
        font_findings.extend(ui_metrics::find_font_findings(&root_node, &patterns, path));
        ui_metrics::find_duplicate_clusters(&root_node, path, &mut duplicate_registry);
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
    let ui_tier = [color_t, spacing_t, font_t, duplicate_t, nesting_t, handler_t]
        .into_iter()
        .max()
        .unwrap();

    Ok(ProjectReport {
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
