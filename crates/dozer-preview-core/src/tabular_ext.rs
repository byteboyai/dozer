//! 表格类数据文件的扩展名判定(纯函数)。从 `dozer-app::tabular` 搬来:路由需要它,而
//! 表格解析器(`calamine` / `csv`)不属于核心。

use std::path::Path;

/// 表格类数据文件,预览时委托 Tabular Viewer(而非代码编辑器 / flyfish)。
pub fn is_tabular_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "xlsx" | "xls" | "ods" | "csv" | "tsv"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tabular_extension_covers_formats_case_insensitive() {
        for p in [
            "/tmp/a.xlsx",
            "/tmp/a.xls",
            "/tmp/a.ods",
            "/tmp/a.csv",
            "/tmp/a.tsv",
            "/tmp/A.CSV",
        ] {
            assert!(is_tabular_extension(Path::new(p)), "{p}");
        }
        for p in ["/tmp/a.md", "/tmp/a.rs", "/tmp/a.png", "/tmp/a.json"] {
            assert!(!is_tabular_extension(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn tabular_ext_is_case_insensitive_and_exact() {
        assert!(is_tabular_extension(Path::new("A.CSV")));
        assert!(is_tabular_extension(Path::new("dir/Report.XlSx")));
        // 扩展名必须整体相等:前缀 / 后缀相似、无扩展名、隐藏文件都不算。
        assert!(!is_tabular_extension(Path::new("a.csvx")));
        assert!(!is_tabular_extension(Path::new("a.xcsv")));
        assert!(!is_tabular_extension(Path::new("a")));
        assert!(!is_tabular_extension(Path::new(".csv")));
        assert!(!is_tabular_extension(Path::new("")));
        assert!(!is_tabular_extension(Path::new("a.csv.bak")));
    }
}
