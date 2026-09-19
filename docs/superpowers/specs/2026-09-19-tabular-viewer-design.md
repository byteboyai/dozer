# Tabular Viewer 设计

## 背景与动机

当前文件预览对表格类数据文件没有专用渲染路径：

- `.csv` / `.tsv` 落在 `preview::is_editable_extension` 的「纯文本兜底」列表里（`crates/dozer-app/src/preview/native_editor.rs` 的 `matches!(ext, "txt" | "log" | ... | "csv" | "tsv")`），被当成普通代码文件塞进原生 `CodeView` 文本编辑器——用户看到的是逗号/制表符分隔的原始文本，没有列对齐、没有表头、没有单元格网格。
- `.xlsx` / `.xls` / `.ods` 不被 `is_editable_extension` 命中，落进 flyfish——flyfish 没有表格渲染管线，最终是「无法预览」或原始字节兜底。

Dozer 的核心原则是「预览优先于编辑」：用户要的是**一眼看清表格数据**（列、行、表头、多 sheet），而不是把表格当文本编辑。因此需要新增一个**只读、高性能**的 Tabular Viewer，专门负责渲染 Excel/CSV/TSV 数据文件，并让系统在打开这些扩展名时把文件委托给它。

## 目标 / 非目标

**目标:**

1. 新增一个 iced 原生的 Tabular Viewer（表格网格），并把 `.xlsx`/`.xls`/`.ods`/`.csv`/`.tsv` 的路由从「代码编辑器 / flyfish」改为「Tabular Viewer」。
2. **高性能**：虚拟化渲染——每一帧只绘制可视窗口内的单元格，行数/列数再多也不随数据总量增长而变慢；内存有上限（载入行数封顶，超出的部分明确提示）。
3. 基于 [Calamine](https://github.com/tafia/calamine) 解析 `.xlsx`/`.xls`/`.ods`，基于 `csv` crate 流式解析 `.csv`/`.tsv`。
4. `.xlsx`/`.ods` 多 sheet 支持：顶部 sheet 切换条，按名称切换。
5. 电子表格式外观：列头（A、B、C…）+ 行号 gutter + 网格线，配色对齐 ByteBoy2077（深/浅两套，随全局 `set_scheme` 自动切换）。
6. 单元格值按类型展示（数字/字符串/布尔/日期），不显示原始 XML/公式。

**非目标（一期裁掉）:**

- 编辑 / 写回单元格（预览即渲染，见 CLAUDE.md 核心原则；不提供任何直接编辑入口）。
- 公式求值（Calamine 对 `.xlsx` 返回的是缓存值，本就无公式依赖）。
- 排序 / 过滤 / 搜索。
- 列宽拖拽调整、冻结行/列、跨单元格区域选择与复制。
- 超出封顶行数的分页 / 增量加载（只显示「前 N 行」并给提示条）。
- `.xlsb`、`.xlsm`（宏）等 Calamine 不支持或需额外处理的格式；`.csv` 用非逗号/非制表符分隔符（分号等）也归入「非目标」，只认标准 `.csv`(逗号) 与 `.tsv`(制表符)。
- 组织/大文件到 SQLite 的落盘缓存（性能优化若真需要，另立计划，不在本计划预支）。

## 数据模型

新模块 `crates/dozer-app/src/tabular/`。数据与视图分离：

```rust
// tabular/mod.rs
/// 一个已打开表格文件的运行时状态(挂在 PreviewTab 上,平行于 editor)。
pub struct TabularView {
    pub sheets: Vec<Sheet>,
    pub active_sheet: usize,
    /// 可视窗口起点(数据行下标 / 数据列下标)。持久化在 state 里,
    /// 每帧由 grid widget 读取;滚动由 Action 消息回写。
    pub scroll_row: usize,
    pub scroll_col: usize,
}

pub struct Sheet {
    pub name: String,
    /// 已载入的单元格(封顶 MAX_TABULAR_ROWS 行)。单元格是"已转成展示
    /// 字符串"的形态,渲染层直接 draw 文本,不做二次解析。
    pub rows: Vec<Vec<String>>,
    /// 列数(取已载入行里最宽的一行;空表为 0)。
    pub col_count: usize,
    /// 真实总行数(封顶前读到多少就记多少;流式 CSV 读满即停,见 loader)。
    pub total_rows: usize,
    /// 是否被截断(总行数超过封顶)。
    pub truncated: bool,
}
```

常量：`MAX_TABULAR_ROWS = 100_000`（一期固定，够绝大多数查看场景；超出即 `truncated = true` 并在顶部渲染提示条「仅显示前 100,000 行，共 N 行」）。

### 单元格值 → 字符串

Calamine 的 `Data` 枚举（`Int`/`Float`/`String`/`Bool`/`DateTime`/`DateTimeIso`/`DurationIso`/`Error`/`Empty`）统一收敛为展示字符串，纯函数 `data_to_string(data: &Data) -> String`：

- `Int` → 整数 `to_string`。
- `Float` → 去尾零的最小表示（`format!("{}", f)` 已是 Rust 最短往返表示，直接用它）。
- `String` → 原文。
- `Bool` → `true`/`false`。
- `DateTime(ExcelDateTime)` / `DateTimeIso` → `YYYY-MM-DD HH:MM:SS`（开 `calamine` 的 `dates` feature 用 chrono 格式化）。
- `Error`/`Empty` → 空串（`Error` 展示为空格，不显示内部错误码，避免表格里满屏 `#REF!` 语义噪音）。
- `DurationIso` → `to_string()` 原文。

CSV/TSV 读进来的本来就是字符串，不做二次类型推断（一期不猜测数字列）。

## 加载器

```rust
// tabular/mod.rs
pub fn load(path: &std::path::Path) -> Result<TabularView, String>
```

按扩展名分派两条路径：

**xlsx/xls/ods（Calamine）**

```rust
let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
for name in &sheet_names {
    if let Ok(range) = workbook.worksheet_range(name) {
        // Range<Data> 的 rows() 迭代器只取前 MAX_TABULAR_ROWS 行
        // col_count 取这些行里的最大列宽
    }
}
```

注意 `worksheet_range` 会把整张 sheet 解析进内存（xlsx 是 zip+xml，这是 Calamine 的语义）。一期接受这个事实：封顶发生在「解析后取前 N 行」这一层，内存峰值由单个 sheet 的实际大小决定，而非由 grid 渲染决定。若后续遇到超大 sheet 撑爆内存的真实案例，再评估 Calamine 的 `RangeDeserializerBuilder` 逐行流式方案（本计划列为非目标，不预支）。

**csv/tsv（`csv` crate 流式）**

```rust
let delimiter = match ext { "tsv" => b'\t', _ => b',' };
let mut rdr = csv::ReaderBuilder::new()
    .delimiter(delimiter)
    .flexible(true)      // 允许参差行(每行列数不一)
    .has_headers(false)  // 表头当普通数据行处理,grid 自画 A/B/C 列头
    .from_path(path)?;
for record in rdr.records().take(MAX_TABULAR_ROWS) {
    // record? -> Vec<String>
}
```

`has_headers(false)`：表头不特殊化——一期把第一行当普通数据（列头用 A/B/C 字母），避免「有的文件有表头、有的没有」的语义猜测；同时保留 `total_rows` 计数到封顶为止。

## 路由（委托）

`crates/dozer-app/src/preview/native_editor.rs` 新增判定：

```rust
/// 表格类数据文件,预览时委托 Tabular Viewer(而非代码编辑器 / flyfish)。
pub fn is_tabular_extension(path: &std::path::Path) -> bool {
    matches!(ext.as_str(), "xlsx" | "xls" | "ods" | "csv" | "tsv")
}
```

同时把 `csv`/`tsv` 从 `is_editable_extension` 的纯文本兜底列表里**移除**（它们不再当文本进代码编辑器）。

`preview::view::push_tab` 的构造逻辑由「二态」变「三态」：

```rust
let editor = match &kind {
    TabKind::File(path) if is_tabular_extension(path) => {
        // 委托 Tabular Viewer:加载表格数据;失败走与 editor 相同的降级(editor
        // 与 tabular 都为 None → 该 tab 落进 webview 兜底,由 flyfish 报「无法
        // 预览」,不在此处造新的错误文案)。
        tabular = tabular::load(path).ok();
        None
    }
    TabKind::File(path) if is_editable_extension(path) && !prefers_rendered_preview(path) => {
        read_and_build_native_editor(path).ok()
    }
    _ => None,
};
```

`PreviewTab` 新增字段 `tabular: Option<tabular::TabularView>`（平行于 `editor`）。关键派生影响：

- **webview 池判定**：`desired_webviews()`、`active_webview_id()`、`select()` 里 `is_webview_file` 的判据从「`editor.is_none()`」改为「`editor.is_none() && tabular.is_none()`」——tabular tab 是 iced 原生渲染，绝不能进 webview 池（否则会被 wry 子视图盖住或误创建 webview）。
- **原生渲染判定**：`active_tab_is_native()` 目前只认 `editor.is_some()`；改为认 `editor.is_some() || tabular.is_some()`。
- **Find/⌘S/撤销重做**：全部只对 `editor` 有意义的动作，天然跳过 tabular tab（它们都判 `editor.is_some()`，tabular 不影响）。
- **主题切换重载**（上一迭代刚加的 `reload_all_webviews_for_theme`）：tabular 是 iced 原生渲染、每帧读 `byteui::theme::color::current()`，切主题自然跟随，**无需**纳入 webview 重载清单（该函数只推进 `editor.is_none()` 的 wry tab，tabular 的 `editor` 为 `None`，会被误推进——需要把它排掉，见 plan）。
- **`wry_toggle_eligible`**：tabular 文件既不 editable 也不 rendered，不出现「预览/代码」切换按钮，天然正确。

## UI 设计

### 网格（`tabular/grid.rs` 自定义 Widget）

电子表格式布局，三层：

1. **列头行**（顶部，冻结）：`A`、`B`、`C`… 列字母，右对齐，背景 `panel`/`tab_active_bg` 区分。
2. **行号 gutter**（左侧，冻结）：`1`、`2`、`3`… 行号，右对齐，同列头背景。
3. **数据区**（可滚）：虚拟化绘制——只 draw 从 `(scroll_row, scroll_col)` 开始、落进当前 viewport 的单元格。

尺寸规则：

- **行高**：`tree_row_font_size()`（与代码编辑器/文件树同公式，跟随全局 scale）+ 固定上下 padding。
- **列宽**：加载时按「前 1000 行」估算每列最大展示宽度（`text` 渲染宽度），钳到 `[MIN_COL_WIDTH, MAX_COL_WIDTH]`（如 `[72px, 240px]`）。估算用前 1000 行而非全量，避免 100k 行 × 宽表的宽度计算开销。列宽存进 `TabularView`（加载后即固定，一期不做拖拽调宽）。
- **总内容尺寸** = `col_count × 列宽` 宽、`total_rows × 行高` 高，供滚动范围计算。

交互（`tabular/grid.rs::Action`）：

```rust
pub enum Action {
    /// 滚轮/触控板滚动,dx/dy 为像素增量。widget 转成行列步进写回 scroll。
    Scroll { dx: f32, dy: f32 },
    /// sheet 切换条点选。
    SelectSheet(usize),
}
```

滚动回写 `TabularView.scroll_row/scroll_col`，widget 每帧读它们计算可视窗口。滚动通过 `iced_widget::mouse::ScrollDelta` 事件处理，横纵双向（纵向按行高步进、横向按列宽步进）。不实现 hover 高亮 / 单元格点选（非目标）。

### sheet 切换条（`tabular/view.rs`）

`TabularView.view()` 组装：顶部一行 sheet 名称 tab（复用 `byteui::interaction::tabs::tab_core` 或现有的 tab 样式，与文件预览 tab 条 / 数据库面板 sheet 条观感一致），下方是网格 widget。单 sheet（csv/tsv）时不画切换条。截断时在切换条与网格之间插一行提示 `text("仅显示前 100,000 行，共 N 行")`（`dim` 色）。

### 配色

全部取 `byteui::theme::color::current()`（`bg`/`panel`/`card`/`border`/`cream`/`body`/`dim`/`gold`）+ `byteui::theme::font::{body,label}`。网格线用 `border`，表头文字用 `cream`，数据文字用 `body`，空单元格不画。深/浅两套随 `set_scheme` 自动生效，无需额外接线。

## 消息与状态

新增 app 级消息（`crates/dozer-app/src/app/message.rs`），沿用 `PreviewFind*` 的「PanelKind 一跳区分 Files/Project」手法：

```rust
/// 表格预览 tab 的交互(滚动/sheet 切换)。`usize` 是 PreviewTab.id,
/// PanelKind 区分 Files / Project 两个预览面板。
TabularAction(PanelKind, usize, tabular::Action),
```

`app/update.rs` 分发到 `Workspace::preview_pane_tabular_action(kind, tab_id, action)`，找到对应 tab 的 `tabular` 并 `apply(action)`。`TabularView::apply` 是纯状态转换（滚动钳到 `[0, 上限]`、`SelectSheet` 钳到合法 sheet 下标），方便单测不依赖真实文件。

## 错误处理

| 场景 | 处理 |
|---|---|
| 文件不存在 / 无权限 / 损坏（Calamine 解析失败） | `tabular::load` 返回 `Err` → `push_tab` 里 `.ok()` 落到 `None` → 该 tab `editor`/`tabular` 双 `None`，走现有 webview/flyfish 兜底（现状已是「打开失败」路径，不新增错误类型）。 |
| `.xlsx` 空表 / 无 sheet | `col_count = 0`、`rows` 空，grid 画空数据区（只有列头/行号/网格框），不崩溃。 |
| CSV 行宽参差 / 含引号、换行、分隔符 | `csv` crate `flexible(true)` 天然处理，`record` 里字段按原样展示（换行显示在单元格内为多行，grid 行高会因此不一致——一期接受：单元格内容只取首行文本、超长省略，见下方「展示裁剪」）。 |
| 单元格文本超长 | 渲染时按列宽裁剪（`text` 加 `.shaping`/宽截断到列宽），超长部分省略，不横向撑破网格。 |
| 超大 sheet 内存 | 一期接受 Calamine 整表解析的语义，仅对行数封顶；遇真实内存问题另立优化计划。 |

## 测试策略

- `data_to_string` 覆盖 Calamine `Data` 每个变体（含 `DateTime`、`Float` 去尾零、`Error`/`Empty`→空串）。
- `is_tabular_extension` 覆盖 5 个扩展名 + 大小写不敏感 + 反例（`.md`/`.rs`/`.png`）。
- `load` 用**真实生成的样本文件**做端到端测试：`calamine` 写一个 `.xlsx`（或借助 `tests/fixtures` 里放置小型 xlsx/csv/tsv 样本），断言 sheet 数、行列数、截断标志、单元格字符串；CSV 覆盖引号/换行/参差行。
- `Sheet` 列宽估算纯函数单测（前 1000 行钳位逻辑）。
- `TabularView::apply` 纯函数单测（滚动钳位、sheet 切换越界 no-op）。
- `push_tab` 路由单测：`.csv`/`.xlsx` 打开后 `tabular.is_some()`、`editor.is_none()`，且 `desired_webviews()` 不含该 tab（不落 webview 池）。
- 大文件性能不做自动化断言（人工验证清单覆盖），但 `MAX_TABULAR_ROWS` 截断路径有单测。

## 依赖

```toml
# crates/dozer-app/Cargo.toml
calamine = { version = "0.36", features = ["dates"] }  # xls/xlsx/ods 全部默认编译
csv = "1"
```

Calamine 0.36 起不再用 feature 门控 xls/xlsx/ods（三格式默认全编），`dates` feature 打开 chrono 日期值供 `data_to_string` 格式化日期。
