# Tabular Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 iced 原生的高性能表格预览 Tabular Viewer，让 `.xlsx`/`.xls`/`.ods`/`.csv`/`.tsv` 打开时被委托给它（而不是代码编辑器 / flyfish）。

**Architecture:** 新模块 `crates/dozer-app/src/tabular/`（`mod.rs` 数据模型+加载器、`grid.rs` 虚拟化网格 Widget、`view.rs` 视图组装）。`PreviewTab` 加 `tabular: Option<TabularView>` 字段，`push_tab` 按 `is_tabular_extension` 三态路由；webview 池判据从「`editor.is_none()`」收紧为「`editor.is_none() && tabular.is_none()`」。

**Tech Stack:** `calamine = { version = "0.36", features = ["dates"] }`（xlsx/xls/ods）、`csv = "1"`（csv/tsv 流式）。网格是自定义 `iced_widget::Widget`（虚拟化 draw + 滚轮事件），配色走 `byteui::theme::color::current()`。

**Spec:** `docs/superpowers/specs/2026-09-19-tabular-viewer-design.md`

---

### Task 1: 依赖 + `is_tabular_extension` + `data_to_string`

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`
- Modify: `crates/dozer-app/src/preview/native_editor.rs`
- New: `crates/dozer-app/src/tabular/mod.rs`（骨架，先放 `data_to_string` 与 `is_tabular_extension`）

**Interfaces:**
- Consumes: 无（新模块）
- Produces: `tabular::is_tabular_extension(&Path) -> bool`、`tabular::data_to_string(&Data) -> String`（Task 2 的 loader 会调用）

- [ ] **Step 1: 加依赖**

`crates/dozer-app/Cargo.toml` `[dependencies]` 追加：

```toml
calamine = { version = "0.36", features = ["dates"] }
csv = "1"
```

- [ ] **Step 2: 编译确认依赖解析成功**

Run: `cargo build -p dozer-app 2>&1 | tail -20`
Expected: 干净编译（新依赖还没被引用，只是确保 registry 能拉到 `calamine 0.36`/`csv 1`，若 `rsproxy` 镜像没有对应版本会在这里暴露，需先处理依赖来源问题）。

- [ ] **Step 3: 注册模块 + 写失败测试**

`crates/dozer-app/src/main.rs`（或 `lib` 结构里）加 `mod tabular;`。在 `tabular/mod.rs` 建 `#[cfg(test)] mod tests`，追加：

```rust
use calamine::Data;

#[test]
fn data_to_string_covers_variants() {
    assert_eq!(data_to_string(&Data::Int(42)), "42");
    assert_eq!(data_to_string(&Data::Float(3.5)), "3.5");
    assert_eq!(data_to_string(&Data::Float(2.0)), "2");
    assert_eq!(data_to_string(&Data::String("x".into())), "x");
    assert_eq!(data_to_string(&Data::Bool(true)), "true");
    assert_eq!(data_to_string(&Data::Empty), "");
    assert_eq!(data_to_string(&Data::Error(calamine::CellErrorType::Div0)), "");
}
```

`is_tabular_extension` 的测试放 `native_editor.rs` 既有 `mod tests`（同文件里 `is_editable_extension` 的测试旁边）：

```rust
#[test]
fn is_tabular_extension_covers_tabular_formats_case_insensitive() {
    for p in ["/tmp/a.xlsx", "/tmp/a.xls", "/tmp/a.ods", "/tmp/a.csv", "/tmp/a.tsv",
              "/tmp/A.CSV"] {
        assert!(crate::preview::is_tabular_extension(Path::new(p)), "{p}");
    }
    for p in ["/tmp/a.md", "/tmp/a.rs", "/tmp/a.png", "/tmp/a.json"] {
        assert!(!crate::preview::is_tabular_extension(Path::new(p)), "{p}");
    }
}
```

- [ ] **Step 4: 跑测试确认编译失败**

Run: `cargo test -p dozer-app tabular`
Expected: `cannot find function 'data_to_string'` / `cannot find function 'is_tabular_extension'`。

- [ ] **Step 5: 实现**

`tabular/mod.rs`：

```rust
//! 表格类数据文件(.xlsx/.xls/.ods/.csv/.tsv)的只读高性能预览:数据模型、
//! 加载器(calamine + csv)、展示字符串转换。渲染见 grid.rs / view.rs。

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

/// Calamine `Data` → 展示字符串。`Error`/`Empty` 收敛为空串(不显示内部
/// 错误码,避免表格满屏 `#REF!` 语义噪音)。日期格式化见 `format_dt`。
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
```

`format_excel_datetime` 用 chrono（`calamine` 的 `dates` feature 让 `Data::DateTime` 携带 `chrono::NaiveDateTime`）：格式 `%Y-%m-%d %H:%M:%S`，日期无时间成分时仅日期。

`native_editor.rs` 里 `is_tabular_extension` 直接转发 `crate::tabular::is_tabular_extension`（或把判定留 `preview` 侧转发，避免两处写死同一份扩展名列表）。

- [ ] **Step 6: 跑测试 + 移除 csv/tsv 从代码编辑器兜底**

Run: `cargo test -p dozer-app tabular` + `cargo test -p dozer-app is_editable`
Expected: 新增测试 PASS。

同时改 `native_editor.rs::is_editable_extension` 兜底列表，把 `"csv" | "tsv"` 移除（改回 `"txt" | "log" | "conf" | "cfg" | "ini"`）。既有测试 `is_editable_extension_covers_common_text_types` 若断言了 `.csv`/`.tsv` 为 editable 需要同步改：现在应断言 `!is_editable_extension(Path::new("data.csv"))` 且 `is_tabular_extension(...)` 为真。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock crates/dozer-app/src/tabular/mod.rs crates/dozer-app/src/preview/native_editor.rs
git commit -m "feat(tabular): 依赖 + is_tabular_extension + data_to_string
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: `tabular::load`（Calamine + csv 流式）

**Files:**
- Modify: `crates/dozer-app/src/tabular/mod.rs`

**Interfaces:**
- Consumes: `calamine::open_workbook_auto`、`csv::ReaderBuilder`、Task 1 的 `data_to_string`
- Produces: `TabularView`/`Sheet` 结构体、`pub fn load(&Path) -> Result<TabularView, String>`

- [ ] **Step 1: 写失败测试（真实样本文件）**

在 `tabular/mod.rs` 的 `mod tests` 追加（用 `calamine` 写出一个 `.xlsx` 样本、手写 `.csv`/`.tsv` 字节）：

```rust
fn write_sample_xlsx(path: &Path) {
    let mut wb = calamine::new_xlsx();
    // 或借 calamine 的 workbook 写 API:见实现时以 0.36 API 为准,
    // 目标:两个 sheet、sheet2 含数字/字符串/空单元格。
}

#[test]
fn load_xlsx_reads_sheets_and_cells() {
    let p = tempfile_sample_xlsx(); // 落到 std::env::temp_dir()
    let v = load(&p).unwrap();
    assert_eq!(v.sheets.len(), 2);
    assert_eq!(v.sheets[0].name, "Sheet1");
    assert_eq!(v.sheets[0].rows[0][0], "1");     // Int
    assert_eq!(v.sheets[0].rows[0][1], "hello"); // String
    assert_eq!(v.sheets[0].rows[0][2], "");      // Empty
}

#[test]
fn load_csv_handles_quotes_and_ragged_rows() {
    // "a,b\"c\",d\nx,y\n" -> 两行,第二行只有两列
    let v = load(&csv_path).unwrap();
    assert_eq!(v.sheets.len(), 1);
    assert_eq!(v.sheets[0].rows[0], vec!["a", "b\"c\"", "d"]);
    assert_eq!(v.sheets[0].rows[1].len(), 2);
    assert_eq!(v.sheets[0].col_count, 3);
}

#[test]
fn load_tsv_uses_tab_delimiter() { /* ... */ }

#[test]
fn load_truncates_at_max_rows() {
    // 造 MAX_TABULAR_ROWS+5 行的 csv,断言 truncated==true、rows.len()==MAX、total_rows 记录真实值
}
```

- [ ] **Step 2: 跑测试确认编译失败**（`TabularView`/`Sheet`/`load` 不存在）

- [ ] **Step 3: 实现**

`Sheet`/`TabularView` 结构体照 spec；`load` 分派：

```rust
pub const MAX_TABULAR_ROWS: usize = 100_000;

pub struct Sheet { pub name: String, pub rows: Vec<Vec<String>>, pub col_count: usize,
                   pub total_rows: usize, pub truncated: bool }

pub struct TabularView { pub sheets: Vec<Sheet>, pub active_sheet: usize,
                         pub scroll_row: usize, pub scroll_col: usize }

pub fn load(path: &Path) -> Result<TabularView, String> {
    match ext { "xlsx" | "xls" | "ods" => load_calamine(path),
                "csv" | "tsv" => load_delimited(path), _ => Err("非表格文件".into()) }
}
```

`load_calamine`：`open_workbook_auto` → 遍历 `sheet_names()` → `worksheet_range(name)` → `rows().take(MAX_TABULAR_ROWS)`，每格 `data_to_string`；`col_count` 取已载入行最大列宽，`total_rows` 用 `range.rows().count()`（封顶前真实行数，注意 `rows()` 借走 range，先 count 再 take 或缓存）。`load_delimited`：按扩展名定 delimiter，`csv::ReaderBuilder` `flexible(true)`/`has_headers(false)`，`records().take(MAX_TABULAR_ROWS)`，`truncated` 通过「是否还有下一行」或 `total_rows` 计数判。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app tabular`
Expected: 新增测试全 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/tabular/mod.rs
git commit -m "feat(tabular): calamine+csv 加载器
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: 列宽估算纯函数

**Files:**
- Modify: `crates/dozer-app/src/tabular/mod.rs`

**Interfaces:**
- Produces: `pub(crate) fn estimate_col_widths(rows: &[Vec<String>], col_count: usize) -> Vec<f32>`（前 1000 行、钳位 `[MIN_COL_WIDTH, MAX_COL_WIDTH]`）

- [ ] **Step 1: 写失败测试**

覆盖：空表 → 空 vec；单列定宽；超长文本钳到 MAX；窄列抬到 MIN；只取前 1000 行（构造 1001 行，第 1001 行有超长值但列宽不受其影响）。

- [ ] **Step 2: 跑测试确认编译失败**

- [ ] **Step 3: 实现**

```rust
pub(crate) const MIN_COL_WIDTH: f32 = 72.0;
pub(crate) const MAX_COL_WIDTH: f32 = 240.0;

pub(crate) fn estimate_col_widths(rows: &[Vec<String>], col_count: usize) -> Vec<f32> {
    let mut max_w = vec![0f32; col_count];
    for row in rows.iter().take(1000) {
        for (c, cell) in row.iter().enumerate().take(col_count) {
            max_w[c] = max_w[c].max(text_measure_width(cell));
        }
    }
    max_w.into_iter().map(|w| w.clamp(MIN_COL_WIDTH, MAX_COL_WIDTH)).collect()
}
```

`text_measure_width` 复用终端/文件树已有的文本测量（找 `tree_row_font_size` / cosmic-text 测量或 `text::Shaping` 的既有封装；若没有现成单串测量函数，退化为「字符数 × 估算字宽」的保守近似并加注释说明，一期可接受）。

- [ ] **Step 4: 跑测试**

- [ ] **Step 5: Commit**

---

### Task 4: `PreviewTab.tabular` 字段 + 三态路由 + webview 池判据收紧

**Files:**
- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/preview/view.rs`

**Interfaces:**
- Consumes: `tabular::load`、`tabular::is_tabular_extension`
- Produces: `PreviewTab.tabular` 字段、`push_tab` 三态构造、`desired_webviews`/`active_webview_id`/`select` 判据收紧、`active_tab_is_native` 含 tabular

- [ ] **Step 1: 写失败测试**

`preview/view.rs` `mod tests` 追加：

```rust
#[test]
fn open_tabular_file_builds_tabular_view_and_excludes_from_webviews() {
    // 写一个真实小 csv
    let p = std::env::temp_dir().join(format!("tabular_route_{}.csv", std::process::id()));
    std::fs::write(&p, "a,b\n1,2").unwrap();
    let mut pane = PreviewPane::default();
    let id = pane.open_path(p.clone());
    let tab = &pane.tabs()[pane.active_idx()];
    assert!(tab.tabular.is_some(), "csv 应构造 TabularView");
    assert!(tab.editor.is_none(), "csv 不应再进代码编辑器");
    assert!(pane.desired_webviews().is_empty(), "tabular tab 不该进 webview 池");
    assert!(pane.active_tab_is_native(), "tabular tab 应是原生 iced 渲染");
    std::fs::remove_file(p).ok();
}
```

- [ ] **Step 2: 跑测试确认编译失败**（`PreviewTab` 无 `tabular` 字段）

- [ ] **Step 3: 实现**

`state.rs` `PreviewTab` 加 `pub tabular: Option<crate::tabular::TabularView>`，`Debug` 加 `.field("tabular", &self.tabular.is_some())`；`placeholder_tab` 与 `push_tab` 构造处补 `tabular: None`。

`view.rs`：

- `push_tab` 构造改三态（见 spec 的代码块）：tabular 分支 `tabular = tabular::load(path).ok()`。
- `desired_webviews` 的 filter 从 `.filter(|(_, tab)| tab.editor.is_none())` 改成 `.filter(|(_, tab)| tab.editor.is_none() && tab.tabular.is_none())`。
- `active_webview_id` 的 `.filter(|t| t.editor.is_none() && matches!(t.kind, TabKind::File(_)))` 加 `&& t.tabular.is_none()`。
- `select` 的 `is_webview_file` 判据同步加 `&& self.tabs[idx].tabular.is_none()`。
- `active_tab_is_native` 改 `t.editor.is_some() || t.tabular.is_some()`。
- `reload_all_webviews_for_theme`（上一迭代加的）加 `&& t.tabular.is_none()`，避免把 tabular tab 误当 webview 推进 nonce。
- `bump_reload` 的 else 分支（`tab.reload_nonce += 1`）加 `&& tab.tabular.is_none()`，或把「webview 才推进」的判据抽成一个 `is_wry_file(tab)` 帮助函数统一使用，避免三处重复。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app preview`
Expected: 新增测试 PASS，既有 `desired_webviews`/`select`/`active_webview_id` 相关测试不受影响。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/preview/state.rs crates/dozer-app/src/preview/view.rs
git commit -m "feat(tabular): PreviewTab 三态路由 + webview 池判据收紧
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: `TabularGrid` 虚拟化网格 Widget

**Files:**
- New: `crates/dozer-app/src/tabular/grid.rs`

**Interfaces:**
- Consumes: `tabular::Sheet`、`byteui::theme::{color,font,geometry}`
- Produces: `pub enum Action { Scroll{dx,dy}, SelectSheet(usize) }`、`pub struct TabularGrid<'a>`（实现 `Widget<Action, Theme, Renderer>`）

- [ ] **Step 1: 实现 Widget**

要点（实现细节以 iced 0.14 实际 API 为准，模式对齐 `code_editor` 的自建 gutter canvas 与 `byteui` 既有自定义 widget）：

- `TabularGrid<'a>` 持有 `&'a Sheet`、`&'a Vec<f32>`（列宽）、`&'a TabularView`（读 `scroll_row/scroll_col`）、`theme`（每帧 `color::current()`）。
- `layout`：内容总宽 = 列宽之和 + gutter；总高 = `total_rows * row_height` + 表头高。返回 `Size`。
- `draw`：算可视行/列窗口 `[scroll_row, scroll_row+rows_in_view)` × `[scroll_col, scroll_col+cols_in_view)`，只对窗口内单元格画 `text` + 网格线 + 表头 + 行号。行号/列头文字用 `cream`、数据用 `body`、线用 `border`、表头背景 `tab_active_bg`。
- `on_event`：`Event::Mouse(Mouse::WheelScrolled { delta, .. })` → 返回 `Some(Action::Scroll { .. })`（把像素增量转成行列步进写回，由 `TabularView::apply` 完成钳位）。其余事件返回 `None`。
- 不处理 hover/点选（非目标）。

滚动语义：纵向按行高步进、横向按列宽步进，用 `iced_widget::mouse::ScrollDelta::Lines/Pixels` 归一化。

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 干净编译（Widget 暂时无调用方，允许未使用告警）。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/tabular/grid.rs
git commit -m "feat(tabular): 虚拟化网格 Widget
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: `TabularView` 视图组装 + `apply`

**Files:**
- New: `crates/dozer-app/src/tabular/view.rs`

**Interfaces:**
- Consumes: `grid::TabularGrid`、`grid::Action`、`byteui::interaction::tabs`
- Produces: `TabularView::apply(&mut self, action)`、`TabularView::view(&self) -> Element<Action, ...>`

- [ ] **Step 1: 写失败测试（`apply` 纯函数）**

覆盖：`Scroll` 正负方向钳到 `[0, 上限]`（上限用 `total_rows`/`col_count` 反推）；`SelectSheet` 越界 no-op、合法切换改 `active_sheet` 并重置 `scroll_row/scroll_col` 到 0。

- [ ] **Step 2: 跑测试确认编译失败**

- [ ] **Step 3: 实现**

`apply` 纯状态转换；`view()` 组装：多 sheet 时顶部 sheet 条（复用 `byteui::interaction::tabs::tab_core`，选中 `active_sheet`，`on_press` 发 `Action::SelectSheet(i)`）+ 截断提示行 + `TabularGrid`（`scrollable` 语义由 widget 内滚轮事件承担，外层不必再套 `scrollable`）。

- [ ] **Step 4: 跑测试**

- [ ] **Step 5: Commit**

---

### Task 7: workspace 渲染 + Message 接线

**Files:**
- Modify: `crates/dozer-app/src/workspace/view.rs`
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`（若 `preview_pane_tabular_action` 放这里）

**Interfaces:**
- Consumes: `tabular::TabularView::view`、`grid::Action`
- Produces: `Message::TabularAction(PanelKind, usize, tabular::Action)`、`Workspace::preview_pane_tabular_action`

- [ ] **Step 1: message.rs 加变体**

```rust
/// 表格预览 tab 交互(滚动/sheet 切换)。usize = PreviewTab.id,PanelKind 区分 Files/Project。
TabularAction(PanelKind, usize, crate::tabular::Action),
```

- [ ] **Step 2: update.rs 分发**

在既有 preview 分发区附近加分支，转发到 `Workspace::preview_pane_tabular_action(kind, tab_id, action)`（找 tab 的 `tabular` 调 `apply`）。需确认 `PanelKind`→「Files 用 `ws.preview`、Project 用 `ws.project_preview`」的映射（同 `PreviewFind*` 的分发写法）。

- [ ] **Step 3: workspace/view.rs 渲染**

在 `preview_pane_for` 的内容渲染处（`if let Some(editor) = &active_tab.editor` 之后）加：

```rust
else if let Some(tabular) = &active_tab.tabular {
    let tab_id = active_tab.id;
    content = container(
        tabular.view().map(move |act| match panel {
            PanelKind::Project => Message::TabularAction(PanelKind::Project, tab_id, act),
            _ => Message::TabularAction(PanelKind::Files, tab_id, act),
        })
    ).into(); // 或 push 进既有 content 列
}
```

`tabular.view()` 需要能拿到 `&mut` 之外还能映射 `Action`；`TabularView::view(&self)` 返回 `Element<Action, ..>`，映射到 app `Message`（`Action` 需 `Clone`/`Debug` 与映射闭包配合，参考 `CodeView::view().map(editor_msg)` 既有写法）。

- [ ] **Step 4: 编译 + 全量测试**

Run: `cargo build -p dozer-app 2>&1 | tail -40` + `cargo test -p dozer-app`
Expected: 干净编译、全部既有 + 本计划新增测试 PASS。

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tabular): workspace 渲染 + Message 接线
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: 全量检查 + 人工验证清单

**Files:** 无代码改动（除非检查发现问题回头小修）

- [ ] **Step 1: 全量构建/测试/静态检查**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全通过；`fmt --check` 报格式就跑 `cargo fmt` 后单独提交格式化 commit。

- [ ] **Step 2: `cargo run -p dozer-app` 人工验证清单**

- [ ] 打开一个 `.csv`（含引号/逗号/多行），显示为网格，表头 A/B/C、行号 1/2/…，列对齐。
- [ ] 打开一个 `.tsv`，列按制表符对齐（不是逗号）。
- [ ] 打开一个多 sheet 的 `.xlsx`，顶部有 sheet 切换条，点选能切换且列头/行号/滚动复位。
- [ ] 打开一个 `.xls`、`.ods`，正常渲染（Calamine 三格式都通）。
- [ ] 打开一个超大 csv（>10 万行），只渲染可视区、滚动流畅不卡 UI，顶部显示「仅显示前 100,000 行」提示。
- [ ] 鼠标滚轮上下/横向（触控板双指）都能滚动，滚动到底/最右不越界。
- [ ] 单元格超长文本被列宽裁剪省略，不横向撑破网格。
- [ ] 切到浅色主题，表格网格随 dozer 全局变浅（表头/文字/网格线颜色跟过去），无需重启。
- [ ] `.csv` 不再进代码编辑器（tab 上没有「预览/代码」切换按钮，没有语法高亮）；`.xlsx` 不再落 flyfish 的「无法预览」。
- [ ] 关闭 tab、项目切换后打开同一文件，滚动位置与 sheet 选择回到初始（不残留）。

- [ ] **Step 3: 若人工验证发现问题,记录并修复**

- [ ] **Step 4: 最终提交(若 Step 3 有修复)**

```bash
git add -A
git commit -m "fix(tabular): 人工验证发现的问题修复
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
