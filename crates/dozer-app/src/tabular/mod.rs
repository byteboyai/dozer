//! 表格类数据文件(.xlsx/.xls/.ods/.csv/.tsv)的只读高性能预览:数据模型、
//! 加载器(calamine + csv)、展示字符串转换、列宽估算。渲染见 grid.rs /
//! view.rs。
//!
//! 性能策略:
//! - 单 sheet 封顶 `MAX_TABULAR_ROWS` 行,且**一旦够数就不再继续解析**——
//!   xlsx/xls/ods 走 calamine 的流式 cell reader(而非一次性把整张 sheet
//!   物化进内存的 `worksheet_range`),csv/tsv 一边读一边数、够数即停,不再
//!   扫到文件末尾。这样打开成本只跟"封顶行数"相关,不跟文件真实大小/
//!   真实行数相关——1GB、几百万行的文件也能快速打开首屏。
//! - 代价:被截断的 sheet 不再报「共 N 行」的精确总数(算精确数就得扫完
//!   全文件,违背上面这条),只标一个「已截断」的近似提示。
//! - 多 sheet 的 xlsx/xls/ods 只在打开时预加载第一个 sheet;其余 sheet
//!   保持 `None`(未加载),切到哪个才在哪个时候才去加载(见 `SheetLoadRequest`)。
//! - 渲染层(grid.rs)只 draw 可视窗口内的单元格——行数/列数再多,单帧开销
//!   只与可视窗口大小相关,不随数据总量增长。

use std::path::{Path, PathBuf};

pub mod grid;
pub mod view;

pub use grid::Action;

/// 单 sheet 载入的行数上限。超出即 `truncated = true`,顶部给提示条,且
/// 加载器一旦读满这个数就不再继续解析文件剩余部分(见模块文档)。
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
/// 派生 `Clone`纯粹是为了满足顶层 `Message` 枚举整体 `#[derive(Clone)]`
/// 的约束(`Message::TabularLoaded` 把它整个搬进消息体,同
/// `database::Message` 对 `Vec<TableRef>` 的既有做法)——不代表这个类型
/// 应该被频繁 clone,实际运行路径里也没有任何地方真的调用 `.clone()`。
#[derive(Debug, Clone)]
pub struct TabularView {
    /// 源文件路径,懒加载其它 sheet 时用来重新打开工作簿。
    path: PathBuf,
    /// 全部 sheet 名(工作簿目录本身很轻,可以一次性拿全);对应下标的
    /// `sheets` 为 `None` 表示这个 sheet 还没被加载过。
    pub sheet_names: Vec<String>,
    pub sheets: Vec<Option<Sheet>>,
    pub active_sheet: usize,
    /// 可视窗口起点(数据行下标 / 数据列下标)。持久在 state 里,每帧由 grid
    /// widget 读取;滚动由 `Action::Scroll` 经 `apply` 回写并钳位。
    pub scroll_row: usize,
    pub scroll_col: usize,
    /// 已经 spawn 出去、还没等到 `apply_sheet_loaded` 回填的 sheet 下标。
    /// 防止用户在一次加载跑完之前来回切走再切回同一个未加载 sheet,
    /// 对着同一份大文件重复 spawn 后台加载(见 `apply` 的 `SelectSheet`
    /// 分支)。
    loading_sheets: std::collections::HashSet<usize>,
}

/// 单个工作表。单元格已是「转成展示字符串」的形态,渲染层直接 draw 文本。
/// sheet 名字不在这里存——那是 `TabularView.sheet_names` 的职责(懒加载
/// 场景下,tab 切换条要在这个 sheet 实际加载完成之前就能显示它的名字)。
/// 派生 `Clone` 的理由同 `TabularView`。
#[derive(Debug, Clone)]
pub struct Sheet {
    /// 已载入单元格,最多 `MAX_TABULAR_ROWS` 行。参差行(CSV)短行缺失的
    /// 单元格渲染为空。
    pub rows: Vec<Vec<String>>,
    /// 列数(取已载入行里最宽的一行;空表为 0)。
    pub col_count: usize,
    /// 已载入的行数(即 `rows.len()`)。`truncated` 为真时这就是显示窗口
    /// 能滚到的行数上限——不是文件的真实总行数(那需要扫完全文件才知道,
    /// 违背"够数即停"的性能策略,见模块文档)。
    pub total_rows: usize,
    /// 是否被截断(文件里还有超出 `MAX_TABULAR_ROWS` 的数据)。
    pub truncated: bool,
    /// 每列展示宽度,单位是「等宽字符格」(unicode 显示宽),渲染层按当前
    /// 字号换算成像素。加载时按前 1000 行估算并钳位,与字号/scale 解耦。
    pub col_widths: Vec<f32>,
}

/// `TabularView::apply` 命中一个尚未加载的 sheet 时返回,交给外层(有
/// handle/proxy 的那一层)去后台加载,完成后经 `apply_sheet_loaded` 回填。
/// `TabularView`/`apply` 自身保持零 IO、纯状态转换,便于单测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetLoadRequest {
    pub index: usize,
    pub path: PathBuf,
    pub name: String,
}

/// 列宽(等宽字符格)上下限。钳位避免超长文本撑破网格、也避免窄列不可读。
pub(crate) const MIN_COL_CELLS: f32 = 8.0;
pub(crate) const MAX_COL_CELLS: f32 = 40.0;

impl TabularView {
    /// 把懒加载的 sheet 列表包成 `TabularView`,初始滚动归零、选中第 0 个
    /// sheet。`sheets[i] = None` 表示第 i 个 sheet 还没加载。
    pub fn new(path: PathBuf, sheet_names: Vec<String>, sheets: Vec<Option<Sheet>>) -> Self {
        Self {
            path,
            sheet_names,
            sheets,
            active_sheet: 0,
            scroll_row: 0,
            scroll_col: 0,
            loading_sheets: std::collections::HashSet::new(),
        }
    }

    /// 当前激活 sheet。`None` 表示 sheet 下标非法(理论上不会:见
    /// `apply` 的边界检查),或该 sheet 尚未加载完成。
    pub fn active_sheet(&self) -> Option<&Sheet> {
        self.sheets.get(self.active_sheet).and_then(|s| s.as_ref())
    }

    /// 处理一条交互动作,纯状态转换(可单测,不做任何 IO)。滚动钳到
    /// `[0, 上限]`;切到一个还没加载的 sheet 时返回 `SheetLoadRequest`,
    /// 交给外层去后台加载(见该类型文档)。
    pub fn apply(&mut self, action: Action) -> Option<SheetLoadRequest> {
        match action {
            Action::Scroll { dx, dy } => {
                let Some(sheet) = self.active_sheet() else {
                    return None; // 当前 sheet 还没加载完,没有边界可钳,不滚动。
                };
                let (max_row, max_col) = (
                    sheet.total_rows.saturating_sub(1),
                    sheet.col_count.saturating_sub(1),
                );
                self.scroll_row = self
                    .scroll_row
                    .saturating_add_signed(dy as isize)
                    .min(max_row);
                self.scroll_col = self
                    .scroll_col
                    .saturating_add_signed(dx as isize)
                    .min(max_col);
                None
            }
            Action::SelectSheet(idx) => {
                if idx >= self.sheet_names.len() || idx == self.active_sheet {
                    return None;
                }
                self.active_sheet = idx;
                self.scroll_row = 0;
                self.scroll_col = 0;
                // 已经有一次加载在飞就不再重复 spawn(2026-09 code review
                // 发现:来回切走再切回同一个未加载 sheet,原来会对着同一份
                // 大文件重复触发后台加载)。
                if self.sheets[idx].is_none() && self.loading_sheets.insert(idx) {
                    Some(SheetLoadRequest {
                        index: idx,
                        path: self.path.clone(),
                        name: self.sheet_names[idx].clone(),
                    })
                } else {
                    None
                }
            }
        }
    }

    /// 一个由 `SheetLoadRequest` 触发的后台加载完成后回填结果。下标越界
    /// (sheet 列表在加载期间发生了变化——目前不会,但防御一下)静默丢弃。
    /// 加载失败时保留 `None`,该 sheet 会一直显示"加载中"占位;失败原因
    /// 记日志供排查,不在 UI 上展示内部错误文案(同 `data_to_string` 对
    /// `Data::Error` 的处理取舍)。无论成功失败都把这个下标从
    /// `loading_sheets` 摘掉——失败时这样用户切走再切回来才能重新触发一次
    /// 加载(没有专门的"重试"按钮,靠这个当退路)。
    pub fn apply_sheet_loaded(&mut self, index: usize, result: Result<Sheet, String>) {
        self.loading_sheets.remove(&index);
        let Some(slot) = self.sheets.get_mut(index) else {
            return;
        };
        match result {
            Ok(sheet) => *slot = Some(sheet),
            Err(err) => tracing::warn!(sheet = index, %err, "表格 sheet 后台加载失败"),
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

/// Excel 序列日期 → 可读字符串。`is_duration()`(如 `[hh]:mm:ss` 时长格式)
/// 只显示时分秒,不带这类值本没有意义的纪元日期前缀;否则零时间分量
/// (00:00:00)时只显示日期,均有时间分量时日期+时间都显示。
fn format_excel_datetime(dt: &calamine::ExcelDateTime) -> String {
    let (y, mo, d, h, mi, s, _ms) = dt.to_ymd_hms_milli();
    if dt.is_duration() {
        format!("{h:02}:{mi:02}:{s:02}")
    } else if h == 0 && mi == 0 && s == 0 {
        format!("{y:04}-{mo:02}-{d:02}")
    } else {
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
    }
}

/// 按扩展名加载表格文件的第一个 sheet(其余 sheet 懒加载,见模块文档)。
/// `.xlsx`/`.xls`/`.ods` 走 Calamine,`.csv`/`.tsv` 走 `csv` crate 流式解析。
pub fn load(path: &Path) -> Result<TabularView, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" | "xls" | "ods" => load_calamine_first_sheet(path),
        "csv" | "tsv" => load_delimited(path, if ext == "tsv" { b'\t' } else { b',' }),
        _ => Err(format!("非表格文件: {ext}")),
    }
}

/// 懒加载单个 sheet(用户切到一个还未加载的 sheet 时,在后台线程调用)。
/// 只有 xlsx/xls/ods 的非首个 sheet 会走到这里——csv/tsv 恒定只有一个
/// sheet,打开时已经加载,不会产生 `SheetLoadRequest`。
///
/// 已知取舍:每次都重新 `open_workbook_auto` 整个工作簿(重新解压/解析
/// 一遍共享字符串表、样式表等元数据),不复用首个 sheet 加载时已经开过的
/// 那份 workbook 句柄——复用需要让 calamine 的 `Sheets<BufReader<File>>`
/// 跨 UI 线程和后台线程的消息边界存活并可变借用,增加的复杂度目前不值得:
/// sheet 切换是低频的显式点击(不在滚动/渲染热路径上),且已经走异步+
/// loading 动画,多花的时间只是这次切换慢一点,不会卡 UI。工作簿元数据
/// 本身通常远小于被封顶的行数据,重复解析的开销有上限。
pub fn load_sheet(path: &Path, name: &str) -> Result<Sheet, String> {
    let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    sheet_from_workbook(&mut workbook, name)
}

fn load_calamine_first_sheet(path: &Path) -> Result<TabularView, String> {
    use calamine::Reader;
    let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let sheet_names = workbook.sheet_names();
    let mut sheets: Vec<Option<Sheet>> = (0..sheet_names.len()).map(|_| None).collect();
    if let Some(first) = sheet_names.first() {
        sheets[0] = Some(sheet_from_workbook(&mut workbook, first)?);
    }
    Ok(TabularView::new(path.to_path_buf(), sheet_names, sheets))
}

/// 加载工作簿里的一个 sheet,读满 `MAX_TABULAR_ROWS` 行就不再继续解析。
/// 仅 xlsx 走 calamine 的流式 cell reader(逐格读,不会像 `worksheet_range`
/// 那样在读之前就把整张 sheet 物化进内存);xls/ods/xlsb 没有等价的流式
/// API,退回 `worksheet_range` 再截断——这三种格式实际不会出现"1GB 级"
/// 文件(xls 硬限 65,536 行,ods/xlsb 用户群和文件规模都远小于 xlsx),
/// 不值得为它们单独实现流式解析。
fn sheet_from_workbook<RS: std::io::Read + std::io::Seek>(
    workbook: &mut calamine::Sheets<RS>,
    name: &str,
) -> Result<Sheet, String> {
    use calamine::{Reader, Sheets};
    match workbook {
        Sheets::Xlsx(xlsx) => sheet_from_xlsx_stream(xlsx, name),
        _ => {
            let range = workbook.worksheet_range(name).map_err(|e| e.to_string())?;
            Ok(sheet_from_range(range))
        }
    }
}

fn sheet_from_xlsx_stream<RS: std::io::Read + std::io::Seek>(
    xlsx: &mut calamine::Xlsx<RS>,
    name: &str,
) -> Result<Sheet, String> {
    let mut reader = xlsx
        .worksheet_cells_reader(name)
        .map_err(|e| e.to_string())?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut col_count = 0usize;
    let mut truncated = false;
    while let Some(cell) = reader.next_cell().map_err(|e| e.to_string())? {
        let (row, col) = cell.get_position();
        let (row, col) = (row as usize, col as usize);
        if row >= MAX_TABULAR_ROWS {
            truncated = true;
            break; // 够数即停:后面即便还有几十万行也不再解析(模块文档)。
        }
        while rows.len() <= row {
            rows.push(Vec::new());
        }
        let line = &mut rows[row];
        if line.len() <= col {
            line.resize(col + 1, String::new());
        }
        line[col] = data_to_string(&calamine::Data::from(cell.get_value().clone()));
        col_count = col_count.max(col + 1);
    }
    let total_rows = rows.len();
    let col_widths = estimate_col_widths(&rows, col_count);
    Ok(Sheet {
        rows,
        col_count,
        total_rows,
        truncated,
        col_widths,
    })
}

/// xls/ods/xlsb 退路:`worksheet_range` 已经把整张 sheet 物化好了,这里
/// 只是截断到封顶行数(见 `sheet_from_workbook` 文档,这三种格式量级小,
/// 不值得单独实现流式解析)。
fn sheet_from_range(range: calamine::Range<calamine::Data>) -> Sheet {
    let (height, width) = range.get_size();
    let truncated = height > MAX_TABULAR_ROWS;
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(height.min(MAX_TABULAR_ROWS));
    for row in range.rows().take(MAX_TABULAR_ROWS) {
        rows.push(row.iter().map(data_to_string).collect());
    }
    let total_rows = rows.len();
    let col_widths = estimate_col_widths(&rows, width);
    Sheet {
        rows,
        col_count: width,
        total_rows,
        truncated,
        col_widths,
    }
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
    let mut truncated = false;
    for record in rdr.records() {
        if rows.len() >= MAX_TABULAR_ROWS {
            truncated = true;
            break; // 够数即停,不再读/解析文件剩余部分(模块文档)。
        }
        let record = record.map_err(|e| e.to_string())?;
        let cells: Vec<String> = record.iter().map(str::to_string).collect();
        col_count = col_count.max(cells.len());
        rows.push(cells);
    }
    let total_rows = rows.len();
    let col_widths = estimate_col_widths(&rows, col_count);
    Ok(TabularView::new(
        path.to_path_buf(),
        vec!["Sheet1".to_string()],
        vec![Some(Sheet {
            rows,
            col_count,
            total_rows,
            truncated,
            col_widths,
        })],
    ))
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
    use std::io::Write;

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
    fn format_excel_datetime_shows_time_only_for_duration() {
        use calamine::{ExcelDateTime, ExcelDateTimeType};
        // 0.604166... 天 = 14:30:00,时长格式(如 [hh]:mm:ss)不该带纪元日期。
        let dt = ExcelDateTime::new(0.604_166_666_666_666_6, ExcelDateTimeType::TimeDelta, false);
        assert_eq!(format_excel_datetime(&dt), "14:30:00");
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

    fn view_with_sheets(sheets: Vec<Option<Sheet>>) -> TabularView {
        let names = (0..sheets.len()).map(|i| format!("s{i}")).collect();
        TabularView::new(PathBuf::from("/tmp/x.xlsx"), names, sheets)
    }

    fn stub_sheet(total_rows: usize, col_count: usize) -> Sheet {
        Sheet {
            rows: vec![],
            col_count,
            total_rows,
            truncated: false,
            col_widths: vec![10.0; col_count],
        }
    }

    #[test]
    fn apply_scroll_clamps_to_loaded_sheet_bounds() {
        let mut v = view_with_sheets(vec![Some(stub_sheet(100, 10))]);
        let req = v.apply(Action::Scroll { dx: 1000, dy: 1000 });
        assert_eq!(req, None);
        assert!(v.scroll_row < 100);
        assert!(v.scroll_col < 10);
    }

    #[test]
    fn apply_scroll_on_unloaded_sheet_is_noop() {
        let mut v = view_with_sheets(vec![None]);
        let req = v.apply(Action::Scroll { dx: 5, dy: 5 });
        assert_eq!(req, None);
        assert_eq!(v.scroll_row, 0);
        assert_eq!(v.scroll_col, 0);
    }

    #[test]
    fn select_sheet_out_of_range_is_noop() {
        let mut v = view_with_sheets(vec![Some(stub_sheet(100, 10))]);
        assert_eq!(v.apply(Action::SelectSheet(9)), None);
        assert_eq!(v.active_sheet, 0);
    }

    #[test]
    fn select_loaded_sheet_switches_without_load_request() {
        let mut v = view_with_sheets(vec![Some(stub_sheet(100, 10)), Some(stub_sheet(5, 3))]);
        v.scroll_row = 50;
        let req = v.apply(Action::SelectSheet(1));
        assert_eq!(req, None);
        assert_eq!(v.active_sheet, 1);
        assert_eq!(v.scroll_row, 0);
        assert_eq!(v.scroll_col, 0);
    }

    #[test]
    fn select_unloaded_sheet_returns_load_request() {
        let mut v = view_with_sheets(vec![Some(stub_sheet(100, 10)), None]);
        let req = v.apply(Action::SelectSheet(1));
        assert_eq!(
            req,
            Some(SheetLoadRequest {
                index: 1,
                path: PathBuf::from("/tmp/x.xlsx"),
                name: "s1".to_string(),
            })
        );
        assert_eq!(v.active_sheet, 1);
        // 切过去了,但这个 sheet 还没数据。
        assert!(v.active_sheet().is_none());
    }

    #[test]
    fn reselecting_still_loading_sheet_does_not_requeue_load() {
        let mut v = view_with_sheets(vec![Some(stub_sheet(100, 10)), None]);
        assert!(
            v.apply(Action::SelectSheet(1)).is_some(),
            "首次切过去应该触发加载"
        );
        v.apply(Action::SelectSheet(0));
        // 第一次加载还没回填(没调 `apply_sheet_loaded`),这次切回去
        // 不应该对着同一个 sheet 再 spawn 一次。
        assert_eq!(
            v.apply(Action::SelectSheet(1)),
            None,
            "同一个 sheet 的加载已经在飞,不该重复触发"
        );
        // 加载结果回来之后,若这个 sheet 之后又被清空(理论上不会,但防御
        // 一下)再切过去应该能重新触发。
        v.apply_sheet_loaded(1, Err("boom".to_string()));
        v.apply(Action::SelectSheet(0));
        assert!(
            v.apply(Action::SelectSheet(1)).is_some(),
            "失败之后应该能重新触发加载(没有专门的重试入口)"
        );
    }

    #[test]
    fn apply_sheet_loaded_fills_pending_slot() {
        let mut v = view_with_sheets(vec![Some(stub_sheet(100, 10)), None]);
        v.apply(Action::SelectSheet(1));
        v.apply_sheet_loaded(1, Ok(stub_sheet(7, 2)));
        let sheet = v.active_sheet().expect("加载完成后应有数据");
        assert_eq!(sheet.total_rows, 7);
        assert_eq!(sheet.col_count, 2);
    }

    #[test]
    fn apply_sheet_loaded_failure_leaves_slot_empty() {
        let mut v = view_with_sheets(vec![None]);
        v.apply_sheet_loaded(0, Err("boom".to_string()));
        assert!(v.active_sheet().is_none());
    }

    #[test]
    fn active_sheet_is_none_for_zero_sheet_workbook() {
        let v = TabularView::new(PathBuf::from("/tmp/empty.xlsx"), vec![], vec![]);
        // 曾经这里会直接越界 panic(`&self.sheets[self.active_sheet]`)。
        assert!(v.active_sheet().is_none());
    }

    #[test]
    fn load_csv_handles_quotes_and_ragged_rows() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.csv");
        std::fs::write(&p, "a,\"b,c\",d\nx,y\n").unwrap();
        let v = load(&p).unwrap();
        assert_eq!(v.sheets.len(), 1);
        let s = v.active_sheet().unwrap();
        assert_eq!(s.rows[0], vec!["a", "b,c", "d"]);
        assert_eq!(s.rows[1].len(), 2);
        assert_eq!(s.col_count, 3);
        assert_eq!(s.total_rows, 2);
        assert!(!s.truncated);
    }

    #[test]
    fn load_csv_stops_at_row_cap_without_scanning_to_eof() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.csv");
        let mut f = std::fs::File::create(&p).unwrap();
        // 封顶之上再多写几行:够数即停时,这些行既不会被存,也不需要被
        // 精确计数(近似统计,见模块文档)。
        for i in 0..MAX_TABULAR_ROWS + 5 {
            writeln!(f, "{i},v{i}").unwrap();
        }
        let v = load(&p).unwrap();
        let s = v.active_sheet().unwrap();
        assert_eq!(s.rows.len(), MAX_TABULAR_ROWS);
        assert_eq!(s.total_rows, MAX_TABULAR_ROWS);
        assert!(s.truncated);
    }

    #[test]
    fn load_tsv_uses_tab_delimiter() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.tsv");
        std::fs::write(&p, "a\tb\n1\t2\n").unwrap();
        let v = load(&p).unwrap();
        let s = v.active_sheet().unwrap();
        assert_eq!(s.rows[0], vec!["a", "b"]);
        assert_eq!(s.rows[1], vec!["1", "2"]);
    }

    #[test]
    fn load_rejects_non_tabular() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.md");
        std::fs::write(&p, "# hi").unwrap();
        assert!(load(&p).is_err());
    }
}
