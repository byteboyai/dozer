//! 表格类数据文件(.xlsx/.xls/.ods/.csv/.tsv)的只读高性能预览:数据模型、
//! 加载器(calamine + csv)、展示字符串转换、列宽估算。渲染见 grid.rs /
//! view.rs。
//!
//! 性能策略:内存封顶 `MAX_TABULAR_ROWS` 行(超出截断并提示),渲染层
//! (grid.rs)只 draw 可视窗口内的单元格——行数/列数再多,单帧开销只与可视
//! 窗口大小相关,不随数据总量增长。

use std::path::Path;

pub mod grid;
pub mod view;

pub use grid::Action;

/// 单 sheet 载入的行数上限。超出即 `truncated = true`,顶部给提示条。
pub const MAX_TABULAR_ROWS: usize = 100_000;

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

/// 一个已打开表格文件的运行时状态(挂在 `PreviewTab` 上,平行于 `editor`)。
pub struct TabularView {
    pub sheets: Vec<Sheet>,
    pub active_sheet: usize,
    /// 可视窗口起点(数据行下标 / 数据列下标)。持久在 state 里,每帧由 grid
    /// widget 读取;滚动由 `Action::Scroll` 经 `apply` 回写并钳位。
    pub scroll_row: usize,
    pub scroll_col: usize,
}

/// 单个工作表。单元格已是「转成展示字符串」的形态,渲染层直接 draw 文本。
pub struct Sheet {
    pub name: String,
    /// 已载入单元格,最多 `MAX_TABULAR_ROWS` 行。参差行(CSV)短行缺失的
    /// 单元格渲染为空。
    pub rows: Vec<Vec<String>>,
    /// 列数(取已载入行里最宽的一行;空表为 0)。
    pub col_count: usize,
    /// 真实总行数(封顶前读到多少记多少)。
    pub total_rows: usize,
    /// 是否被截断(总行数超过封顶)。
    pub truncated: bool,
    /// 每列展示宽度,单位是「等宽字符格」(unicode 显示宽),渲染层按当前
    /// 字号换算成像素。加载时按前 1000 行估算并钳位,与字号/scale 解耦。
    pub col_widths: Vec<f32>,
}

/// 列宽(等宽字符格)上下限。钳位避免超长文本撑破网格、也避免窄列不可读。
pub(crate) const MIN_COL_CELLS: f32 = 8.0;
pub(crate) const MAX_COL_CELLS: f32 = 40.0;

impl TabularView {
    /// 把载入好的 sheet 列表包成 `TabularView`,初始滚动归零、选中第 0 个 sheet。
    pub fn new(sheets: Vec<Sheet>) -> Self {
        Self {
            sheets,
            active_sheet: 0,
            scroll_row: 0,
            scroll_col: 0,
        }
    }

    /// 当前激活 sheet(内部保证 `active_sheet` 合法)。
    pub fn active_sheet(&self) -> &Sheet {
        &self.sheets[self.active_sheet]
    }

    /// 处理一条交互动作,纯状态转换(可单测)。滚动钳到 `[0, 上限]`。
    pub fn apply(&mut self, action: crate::tabular::grid::Action) {
        use crate::tabular::grid::Action;
        match action {
            Action::Scroll { dx, dy } => {
                self.scroll_row = self.scroll_row.saturating_add_signed(dy as isize);
                self.scroll_col = self.scroll_col.saturating_add_signed(dx as isize);
                let (max_row, max_col) = {
                    let sheet = self.active_sheet();
                    (
                        sheet.total_rows.saturating_sub(1),
                        sheet.col_count.saturating_sub(1),
                    )
                };
                if self.scroll_row > max_row {
                    self.scroll_row = max_row;
                }
                if self.scroll_col > max_col {
                    self.scroll_col = max_col;
                }
            }
            Action::SelectSheet(idx) => {
                if idx < self.sheets.len() && idx != self.active_sheet {
                    self.active_sheet = idx;
                    self.scroll_row = 0;
                    self.scroll_col = 0;
                }
            }
        }
    }
}

/// Calamine `Data` → 展示字符串。`Error`/`Empty` 收敛为空串(不显示内部
/// 错误码,避免表格满屏 `#REF!` 语义噪音)。日期/时间格式化为 `YYYY-MM-DD
/// HH:MM:SS`,时间分量为零(纯日期)时只显示日期。
pub fn data_to_string(data: &calamine::Data) -> String {
    use calamine::Data;
    match data {
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(), // Rust 最短往返表示,2.0 显示为 "2"
        Data::String(s) => s.clone(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => format_excel_datetime(dt),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(_) | Data::Empty => String::new(),
    }
}

/// Excel 序列日期 → 可读字符串。零时间分量(00:00:00)时只显示日期。
fn format_excel_datetime(dt: &calamine::ExcelDateTime) -> String {
    let (y, mo, d, h, mi, s, _ms) = dt.to_ymd_hms_milli();
    if h == 0 && mi == 0 && s == 0 {
        format!("{y:04}-{mo:02}-{d:02}")
    } else {
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
    }
}

/// 按扩展名加载表格文件。`.xlsx`/`.xls`/`.ods` 走 Calamine,`.csv`/`.tsv`
/// 走 `csv` crate 流式解析。
pub fn load(path: &Path) -> Result<TabularView, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" | "xls" | "ods" => load_calamine(path),
        "csv" | "tsv" => load_delimited(path, if ext == "tsv" { b'\t' } else { b',' }),
        _ => Err(format!("非表格文件: {ext}")),
    }
}

fn load_calamine(path: &Path) -> Result<TabularView, String> {
    use calamine::Reader;
    let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let names = workbook.sheet_names();
    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let range = workbook.worksheet_range(&name).map_err(|e| e.to_string())?;
        let (height, width) = range.get_size();
        let truncated = height > MAX_TABULAR_ROWS;
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(height.min(MAX_TABULAR_ROWS));
        for row in range.rows().take(MAX_TABULAR_ROWS) {
            rows.push(row.iter().map(data_to_string).collect());
        }
        let col_widths = estimate_col_widths(&rows, width);
        sheets.push(Sheet {
            name,
            rows,
            col_count: width,
            total_rows: height,
            truncated,
            col_widths,
        });
    }
    Ok(TabularView::new(sheets))
}

fn load_delimited(path: &Path, delimiter: u8) -> Result<TabularView, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true) // 允许参差行(每行列数不一)
        .has_headers(false) // 表头当普通数据行,grid 自画 A/B/C 列头
        .from_path(path)
        .map_err(|e| e.to_string())?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut col_count = 0usize;
    let mut total_rows = 0usize;
    for record in rdr.records() {
        total_rows += 1;
        // 超过封顶仍继续计数(为了准确的「共 N 行」),但不再存内容。
        if rows.len() < MAX_TABULAR_ROWS {
            let record = record.map_err(|e| e.to_string())?;
            let cells: Vec<String> = record.iter().map(str::to_string).collect();
            col_count = col_count.max(cells.len());
            rows.push(cells);
        }
    }
    let truncated = total_rows > MAX_TABULAR_ROWS;
    let col_widths = estimate_col_widths(&rows, col_count);
    Ok(TabularView::new(vec![Sheet {
        name: "Sheet1".to_string(),
        rows,
        col_count,
        total_rows,
        truncated,
        col_widths,
    }]))
}

/// 按「前 1000 行」估算每列展示宽度(等宽字符格),钳到 `[MIN_COL_CELLS,
/// MAX_COL_CELLS]`。只取前 1000 行避免大表 O(全表) 的宽度测算;换行按
/// 各物理行取最大。纯函数,便于单测。
pub(crate) fn estimate_col_widths(rows: &[Vec<String>], col_count: usize) -> Vec<f32> {
    let mut max_cells = vec![0f32; col_count];
    for row in rows.iter().take(1000) {
        for (c, cell) in row.iter().enumerate() {
            let w = cell
                .split('\n')
                .map(|line| unicode_width::UnicodeWidthStr::width(line) as f32)
                .fold(0.0f32, f32::max);
            if w > max_cells[c] {
                max_cells[c] = w;
            }
        }
    }
    max_cells
        .into_iter()
        .map(|w| w.clamp(MIN_COL_CELLS, MAX_COL_CELLS))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::Data;

    #[test]
    fn data_to_string_covers_variants() {
        assert_eq!(data_to_string(&Data::Int(42)), "42");
        assert_eq!(data_to_string(&Data::Float(3.5)), "3.5");
        assert_eq!(data_to_string(&Data::Float(2.0)), "2");
        assert_eq!(data_to_string(&Data::String("x".into())), "x");
        assert_eq!(data_to_string(&Data::Bool(true)), "true");
        assert_eq!(data_to_string(&Data::Empty), "");
        assert_eq!(
            data_to_string(&Data::Error(calamine::CellErrorType::Div0)),
            ""
        );
    }

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
    fn estimate_col_widths_clamps_and_measures_cjk() {
        let rows = vec![
            vec!["abc".to_string(), "你好".to_string()],
            vec!["12345".to_string()],
        ];
        let w = estimate_col_widths(&rows, 2);
        assert_eq!(w.len(), 2);
        // 第一列最宽 "12345" = 5 格,但被 MIN_COL_CELLS=8 抬到 8。
        assert_eq!(w[0], MIN_COL_CELLS);
        // 第二列 "你好" = 2 个 CJK = 4 格,同样被抬到 8。
        assert_eq!(w[1], MIN_COL_CELLS);
    }

    #[test]
    fn estimate_col_widths_caps_at_max() {
        let long = "x".repeat(200);
        let rows = vec![vec![long]];
        let w = estimate_col_widths(&rows, 1);
        assert_eq!(w[0], MAX_COL_CELLS);
    }

    #[test]
    fn apply_scroll_clamps_and_select_sheet_resets() {
        let mut v = TabularView::new(vec![
            Sheet {
                name: "a".into(),
                rows: vec![],
                col_count: 10,
                total_rows: 100,
                truncated: false,
                col_widths: vec![10.0; 10],
            },
            Sheet {
                name: "b".into(),
                rows: vec![],
                col_count: 3,
                total_rows: 5,
                truncated: false,
                col_widths: vec![10.0; 3],
            },
        ]);
        use crate::tabular::grid::Action;
        v.apply(Action::Scroll { dx: 1000, dy: 1000 });
        assert!(v.scroll_row < 100);
        assert!(v.scroll_col < 10);
        // 越界 sheet 选择 no-op
        v.apply(Action::SelectSheet(9));
        assert_eq!(v.active_sheet, 0);
        // 合法切换复位滚动
        v.apply(Action::SelectSheet(1));
        assert_eq!(v.active_sheet, 1);
        assert_eq!(v.scroll_row, 0);
        assert_eq!(v.scroll_col, 0);
    }

    #[test]
    fn load_csv_handles_quotes_ragged_rows_and_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.csv");
        std::fs::write(&p, "a,\"b,c\",d\nx,y\n").unwrap();
        let v = load(&p).unwrap();
        assert_eq!(v.sheets.len(), 1);
        let s = v.active_sheet();
        assert_eq!(s.rows[0], vec!["a", "b,c", "d"]);
        assert_eq!(s.rows[1].len(), 2);
        assert_eq!(s.col_count, 3);
        assert_eq!(s.total_rows, 2);
        assert!(!s.truncated);
    }

    #[test]
    fn load_tsv_uses_tab_delimiter() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.tsv");
        std::fs::write(&p, "a\tb\n1\t2\n").unwrap();
        let v = load(&p).unwrap();
        assert_eq!(v.active_sheet().rows[0], vec!["a", "b"]);
        assert_eq!(v.active_sheet().rows[1], vec!["1", "2"]);
    }

    #[test]
    fn load_rejects_non_tabular() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.md");
        std::fs::write(&p, "# hi").unwrap();
        assert!(load(&p).is_err());
    }
}
