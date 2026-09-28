# Tabular 预览渲染层迁移到 WebView 设计

## 背景与动机

现有 Tabular Viewer(见 [`2026-09-19-tabular-viewer-design.md`](./2026-09-19-tabular-viewer-design.md))用纯 iced canvas 手绘表格网格(`crates/dozer-app/src/tabular/grid.rs`),是目前所有预览类型里唯一还没有走"webview 承载预览内容"这条路的——CodeMirror(代码/文本)、vanilla-jsoneditor(严格 JSON)、`dozer://html` sandbox(HTML)都已经是 webview host。

手绘 canvas 网格要做到视觉上精致(阴影、hover、圆角、动效)成本很高,而 JS 表格库把这些做成了开箱即用的能力。这次迁移的目标是**用更少的手工样式成本获得更好看的表格观感**,同时不退化现有的性能特性(百万行文件秒开首屏)与 agent 导航能力(`preview_navigate` 的 reveal cell/range)。

评估过用 [opencalc](https://github.com/CasualOffice/opencalc)(可嵌入、支持读写 xlsx 的 Rust 电子表格引擎)整体替换,结论是不合适:它的定位是"公式引擎 + 可编辑电子表格",编辑器是 WASM canvas,与现有 iced 架构和"预览优先于编辑"的核心原则不匹配,且为了公式正确性/保真度会牺牲现有这套"够数即停、不扫全文件"的性能捷径。因此本次范围**只换渲染层**,不引入新的表格引擎,数据加载/解析层(`calamine` + `csv`)完全不变。

## 目标 / 非目标

**目标:**

1. 把 tabular 预览的**整个面板**(sheet 切换条 + 截断提示条 + 单元格网格)从 iced 原生控件迁移到 webview host,新增 `dozer://tabular/` host,复用现有三种 host(CodeMirror/Flyfish/vanilla-jsoneditor)共用的 `WebviewEnvelope<T>` 消息 envelope 模式。
2. 选用 [ag-grid Community](https://www.ag-grid.com/)(MIT)作为渲染库,利用其免费层自带的 Infinite Row Model 实现"虚拟滚动条按总行数渲染、JS 只持有当前窗口数据"的窗口化能力。
3. 保留现有性能特性:单 sheet 封顶 `MAX_TABULAR_ROWS`(10 万行)、calamine 流式读取/csv 边读边数够数即停、大文件加载可取消(`TABULAR_CANCELLED`)——这些都在 `tabular/mod.rs` 的加载层,本次不动。
4. 保留 agent 导航能力:`reveal_cell`/`reveal_range`(MCP `preview_navigate` 的落点)在新架构下继续可用,效果等价(切 sheet + 滚动到位 + 高亮)。
5. 保留会话持久化:`PersistedTabular`(scroll 位置、active sheet)重开 tab 后能恢复。
6. 顺带开启纯视觉能力:冻结表头行(Excel 风格列字母 A/B/C)+ 冻结首列(行号 gutter)+ 斑马纹。这些是 ag-grid 免费层内建选项,不需要额外开发。
7. 直接切换:新 host 完工即删除旧 iced canvas 实现,不留两套并存的对照期(tabular 预览的使用频率/复杂度远低于 JSON viewer,当初 vanilla-jsoneditor 走对照期的理由在这里不成立)。

**非目标:**

- **不引入编辑能力**。ag-grid 网格选项恒 `editable: false`;选中/复制文本不受影响(等价 CodeMirror 只读模式下仍可复制)。这是核心裁决,不因为换了更"能干"的表格库就顺带开权限。
- **不开排序 / 过滤**。这类交互会重排视图行序,与 agent reveal 依赖的"行列坐标 = 数据坐标"假设冲突,即使 ag-grid 免费层自带也不开。
- **不改变数据加载/解析层**。`tabular/mod.rs` 的 `load`/`load_cancellable`/`load_sheet*`/`Sheet`/`select_sheet` 保持不变,`MAX_TABULAR_ROWS`、截断策略、取消机制原样保留。
- **不做列宽拖拽调整、跨单元格区域选择与复制**(沿用上一版非目标,ag-grid 免费层虽然支持,但不在这次范围内,YAGNI)。
- **不用 opencalc 或任何第三方公式/电子表格引擎**替换现有解析层(见"背景与动机"的评估结论)。

## 已发现的硬约束

调研代码时确认的几条约束,直接决定了技术方案的取舍空间:

1. **严格 CSP,无网络**:现有 host(CodeMirror/vanilla-jsoneditor/HTML sandbox)统一 `default-src 'none'`,所有 JS/CSS 必须本地打包进二进制,不能引 CDN。ag-grid Community 需要以本地 npm 依赖 + esbuild 打包的方式接入,和 `web/json-editor` 现有做法一致。
2. **agent reveal 是真实通路**:`reveal_cell`/`reveal_range`(`crates/dozer-app/src/preview/view.rs:1858-1881`)被 MCP `preview_navigate` 调用,不是死代码,新架构必须接住。
3. **数据已整份常驻内存,不是懒加载**:`Sheet.rows: Vec<Vec<String>>` 在 sheet 加载完成后就整份(最多 10 万行)常驻 Rust 内存,`grid.rs` 只是**渲染层**按可视窗口取子集绘制,不是"按需读盘"。因此新协议要解决的是"如何把已有数据搬到 JS 堆"而不是"如何避免读盘"。
4. **只有 push 模型有先例支撑活状态数据**:vanilla-jsoneditor 的 `fetch()` 模式读的是磁盘原始文件字节,不经过 Rust 侧解析结果;而 tabular 的展示字符串是 `calamine`/`csv` 转换后只存在于 iced 单线程拥有的 `AppState` 里的活数据,协议处理线程不能安全地同步读取它。因此必须走"JS 请求 → update 循环计算 → IPC 推回"的 push 模型,与 CodeMirror 窗口化文本的路数一致。
5. **`Sheet` 已是全量数据的切片源,窗口滑动零 IO**:和大文本窗口化(需要磁盘稀疏索引)不同,tabular 的"下一个窗口"只是对已在内存的 `Vec<Vec<String>>` 做切片,没有索引/磁盘开销,协议可以比文本窗口化更简单。

## 技术方案

### 库选型:ag-grid Community + Infinite Row Model

选它不是因为"好看"(几家开源表格库都能做到),而是**Infinite Row Model 直接对应现有的窗口化语义**:虚拟滚动条按 `total_rows` 渲染,JS 侧只通过 `getRows(params)` 回调按需持有当前窗口的行,不用手写"总数很大、只加载一部分"这套虚拟化逻辑。冻结表头/首列、斑马纹都是免费层内建的 grid option。

备选 Tabulator(MIT,更轻量)被否掉:它没有等价于 Infinite Row Model 的"总数大、按块加载"现成语义,要自己拼,工作量和这次想省下来的"网格出效果"精力是同一块,不划算。

### 协议:`TabularCommand` / `TabularEvent`

新增到 `webview_protocol.rs`,作为第 4 种 host,复用 `WebviewEnvelope<T>`:

**Rust → JS(`TabularCommand`)**

| 命令 | 用途 |
|---|---|
| `Init { sheet_names, active_sheet, read_only }` | `Ready` 后立即发,JS 据此画整条 sheet tab 栏(未加载的 sheet 也要能点,保留现有行为) |
| `SetSchema { sheet_index, col_count, total_rows, truncated, col_widths }` | 某 sheet 可显示时发,JS 用它配置 ag-grid 列定义 + 截断提示条 |
| `SetSheetLoading { sheet_index, loading }` | 懒加载中,JS 把对应 tab 标 loading、网格区显示 spinner(替代原 `loading_hint` iced 组件,这块视觉挪进 host) |
| `SetWindow { sheet_index, start_row, rows, revision }` | 响应 `WindowRequest`;`revision` 丢弃过期响应(同文本窗口化) |
| `RevealRange { sheet_index, r1, c1, r2, c2 }` | agent reveal:切 sheet(未加载先走 `SheetLoadRequest`)+ 滚动到位 + 高亮 |
| `RestoreViewState { sheet_index, scroll_row, scroll_col, selection }` | 首个窗口应用后发,回填 `PersistedTabular` |

**JS → Rust(`TabularEvent`)**

| 事件 | 用途 |
|---|---|
| `Ready` | host 初始化完成 |
| `SheetSelected { index }` | 用户点了 sheet tab(原 iced `Action::SelectSheet` 的接棒者) |
| `WindowRequest { sheet_index, start_row, end_row }` | ag-grid Infinite Row Model 的 `getRows` 回调发出 |
| `WindowApplied { start_row }` | 首个窗口真正挂上,Rust 以此为 Ready 边界(host 保持 hidden 直到这条到,避免"空表格+行号1"闪烁) |
| `Failed { message, recoverable }` | 加载/渲染失败 |

### Host 页面

新增 `dozer://tabular/` URL 前缀,`TabularHostBinding`(`crates/dozer-app/src/preview/tabular_host.rs`)照抄 `code_host.rs::EditorHostBinding` 的结构(project_id/panel/tab_id/path → `document_id`)。URL 只带 `theme=`,**不带** `fs`/`lh`(那是 CodeMirror 专用的 JetBrains Mono 字号参数)——tabular host 走系统默认字体,对齐 CLAUDE.md"非代码/终端场景禁用等宽字体"的裁决。

只读:ag-grid `editable: false`,不启用 `sortable`/`filter`。视觉:冻结表头行(A/B/C 列字母)+ 冻结首列(行号 gutter,用 Infinite Row Model 的绝对 `rowIndex` 而不是块内下标,避免翻页后行号错位)+ 斑马纹,配色走 ByteBoy2077 CSS 变量。

### 迁移(删除 / 新增 / 改造)

沿用现有 host 目录约定:`crates/dozer-app/web/<host>/` 源码(esbuild)构建到 `crates/dozer-app/assets/<host>/`,`assets.rs` 按 `CARGO_MANIFEST_DIR` 路径 serve。

**新增**

- `crates/dozer-app/web/tabular-host/`:`package.json`(依赖 `ag-grid-community`,固定版本)、`build.mjs`、`src/index.html`/`main.ts`/`theme.css`。
- `crates/dozer-app/assets/tabular-host/`:构建产物 + `assets.rs` 新增匹配分支 + CSP。
- `crates/dozer-app/src/preview/tabular_host.rs`:`TabularHostBinding` + URL 构造。
- `webview_protocol.rs`:`TabularCommand`/`TabularEvent` + `parse_tabular_event`。

**改造**

- `preview/router.rs`、`preview/state.rs`:`PreviewKind::Tabular` 的 backend 从 iced widget 改为 webview host 分支;新增 `uses_tabular_host()` 判定(不复用 `uses_editor_host()`——tabular 恒只读、恒不 dirty,save_gate 语义不同)。
- `preview/view.rs`:`reveal_cell`/`reveal_range` 落点从"改 `TabularView.selection` 供 iced 重绘"改为"入队 `RevealRange` command"(同 `RevealPosition` 的 IPC 派发路)。
- `workspace/state.rs`:sheet 懒加载触发点(`SheetLoadRequest`)不变,完成后从"回填触发 iced 重绘"改为"回填 + 发 `SetSchema`/`SetWindow`/`SetSheetLoading(false)`"。
- `preview_state.rs::PersistedTabular`:恢复路径接到 `RestoreViewState` command。
- `tabular/mod.rs`:保留 `load`/`load_cancellable`/`load_sheet*`/`Sheet`/`select_sheet`/`reveal_cell`/`reveal_range`(数据模型/加载逻辑不变);去掉 `Action::Scroll`/`Action::SelectSheet` 及 `apply()` 对应分支(交互不再发生在 iced 侧)。

**删除**

- `crates/dozer-app/src/tabular/grid.rs`(canvas 手绘网格,含 `column_letter` 等——等价能力由 ag-grid 内建列头提供)。
- `crates/dozer-app/src/tabular/view.rs`(sheet tab 栏 + 截断提示的 iced 组装)。
- `app/message.rs::Message::TabularAction`(不再有 iced 交互消息)。

### 落地顺序

直接切换、不留对照期,但落地本身分步验证:

1. JS host 骨架(`Init`/`SetSchema`/`SetWindow` 假数据联调)+ 只读/冻结/斑马纹视觉过一遍。
2. Rust 协议 + `tabular_host.rs` + `assets.rs` 接线,接上真实 `load`/`load_sheet`,懒加载/截断/取消三条既有路径逐一验证。
3. `RevealRange`/`RestoreViewState` 接 agent reveal 与会话持久化,两条既有测试(`reveal_cell`/`reveal_range`/`PersistedTabular`)保持绿。
4. 删除 `grid.rs`/`view.rs`/`Action`,清理死代码,`cargo clippy --all-targets`。

## 测试策略

- Rust 侧协议解析、`WindowRequest → SetWindow` 切片逻辑走单测(纯函数,同 `webview_protocol.rs` 现有测试风格)。
- `select_sheet`/`reveal_cell`/`reveal_range` 现有单测原样保留(数据模型不变)。
- JS 侧只做 `typecheck` + 人工过一遍,不为这次迁移新增 JS 测试基建。
- 落地后在真实 GUI 里过一遍:大文件截断、多 sheet 懒加载、agent reveal、关闭重开(持久化)四条路径。

## 风险

- ag-grid Community 打包体积比手写 canvas 网格大,一次性嵌入二进制,可接受但需在实现时确认不显著拉长构建/启动时间。
- 直接切换意味着没有对照期兜底,依赖第 4 步之前的分阶段验证 + GUI 人工验收控住风险。

## 参考

- 前身:[`2026-09-19-tabular-viewer-design.md`](./2026-09-19-tabular-viewer-design.md)(数据模型/加载层继续有效)
- 架构总规格:[`2026-09-22-file-preview-architecture-redesign.md`](./2026-09-22-file-preview-architecture-redesign.md)
- 窗口化协议先例:[`2026-09-19-large-file-editor-performance-design.md`](./2026-09-19-large-file-editor-performance-design.md)、[`2026-09-24-preview-command-channel-design.md`](./2026-09-24-preview-command-channel-design.md)
