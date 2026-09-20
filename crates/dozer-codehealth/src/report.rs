use crate::function_metric::{FunctionMetric, Severity};
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
    let mut files = Vec::new();
    if root.is_dir() {
        collect_rs_files(root, &mut files)?;
    }

    let mut all_functions = Vec::new();
    let mut total_loc = 0usize;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            // 单文件读不到/非 UTF-8 → 跳过,不中断整体扫描(见 spec「错误处理」)。
            continue;
        };
        total_loc += src.lines().count();
        all_functions.extend(crate::function_metric::functions_in_source(&src, path));
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

    Ok(ProjectReport {
        total_loc,
        total_functions,
        critical_functions,
        scale_tier: scale,
        density_tier: density,
        overall_tier: scale.max(density),
        functions,
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
}
