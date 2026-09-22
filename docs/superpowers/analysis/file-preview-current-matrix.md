# 文件预览现状迁移矩阵(Phase A 冻结)

> 目的:在开始统一 backend 迁移前,把当前 `PreviewTab` 的组合状态、调用链和
> 已知失败固定下来,作为"迁移不得改变用户可见行为"的对照基线。
>
> 对应计划:`docs/superpowers/plans/2026-09-22-file-preview-foundation.md`
> Task 1。本文件只描述**改动前**的现状;Phase A 已落地的 `route/backend`
> 描述见 `crates/dozer-app/src/preview/{router,backend,file_profile}.rs`。

## 1. 两条 pane 的完整调用链

Files 用 `Workspace::preview`,Project 右配对用 `Workspace::project_preview`,
两者是**独立**的 `PreviewPane` 实例(见 `workspace/state.rs` 字段注释),所有
下述链路都各跑一份,只是入口不同。

### 打开
- Files:`App::preview_open_path` → `preview_open_path_at`
  (`app/update.rs:3619`)。
- Project:`App::project_preview_open_path`(`app/update.rs:3807`)。
- 共同逻辑:文件不存在 → 写 `preview_error`;写 `allowed_files` 白名单;
  `find_existing_file_tab` 命中则 `select` 复用;否则
  - `is_native_editor_candidate(path)`(=`!tabular && is_editable_extension
    && !prefers_rendered_preview`,见 `preview/view.rs:37`)为真:走**异步**
    路径 `PreviewPane::insert_loading_tab` + `spawn_blocking
    read_and_build_native_editor` → `Message::PreviewFileLoaded` /
    `ProjectPreviewFileLoaded` → `PreviewPane::apply_native_load`;
  - 否则:`PreviewPane::open_path` → 同步 `push_tab`(表格登记
    `pending_tabular_loads`、JSON 登记 `pending_json_tree_loads`、渲染/媒体
    交给 wry)。
- 收尾:调用方 `take_pending_tabular_loads` / `take_pending_json_tree_loads`
  并 spawn 后台加载;`tab_window_reveal` 滚出激活 tab;
  `spawn_preview_state_save`;`spawn_preview_context_push`。

### 渲染
- `workspace/view.rs::preview_pane_for`(约 900–1400 行)按 tab 的
  `editor` / `json_tree` / `tabular` / `Blank` 分派:
  - `editor`(且非 JSON) → 原生 `CodeView`;
  - `json_tree` → JSON 树 / 原文双视图(顶部 `tab_json_tree_mode_button`);
  - `tabular` → Tabular Viewer 网格;
  - `Blank` → 项目根简介卡;
  - 其余 → wry/flyfish 或 `file://` HTML(见 `preview/webview.rs::preview_url`)。
- Markdown/HTML 的「预览/代码」切换按钮判据 `wry_toggle_eligible`
  (`workspace/view.rs:1057`)。

### 搜索
- ⌘F/⌘R:`Message::PreviewFindOpen(WithReplace)`(`app/update.rs:1095` /
  `1121`)按激活 tab 的 `editor.is_read_only()` 二选一:
  - 原生可写 editor → `PreviewPane::open_find_on_active`(iced Find 条,
    `find_matches_all` 现算;webview 档走 flyfish `searchDocument`);
  - 只读大文件档 → `open_large_file_search`(磁盘流式扫描
    `extensions::search`)。

### 保存
- ⌘S:`Message::PreviewSaveActive(kind)` → `Workspace::preview_pane_save_active`
  (`app/update.rs:1076`)。保存后 `clear_dirty_by_id`,或
  `bump_reload`(重建编辑器 / 推进 wry `reload_nonce`)。

### 关闭 / 切换
- `PreviewPane::close` / `select` / `reorder` / `clear_all`(项目切换);
  关闭或切换导致 Find 失配 → `cull_stale_find`。

### 主题
- `Message::Settings(ThemeSelected)` → `reload_all_webviews_for_theme`
  (`app/update.rs:2283`,实现 `preview/view.rs:1297`):把**需要 webview**
  的 tab 推进 `reload_nonce`,URL 加 `_r=` 触发 `sync_webview_pool` 重新
  `load_url`(flyfish `theme` 是 URL 参数,不重载不换色)。

### WebView 池
- `App::preview_desired`(`app.rs:2916` 附近)读
  `PreviewPane::desired_webviews` → `webview_geometry` 算矩形 →
  `runtime::sync_webview_pool` 做差集增删 `load_url`/`set_bounds`/`set_visible`。
- `active_webview_id` 决定"当前激活的是不是真 webview"(焦点/⌘F 路由用)。

### MCP / Agent 上下文
- `Workspace::spawn_preview_context_push`(`workspace/state.rs:1199`)从激活
  原生 editor 读 `cursor_position`/`selection_range`(经
  `preview_context_from_editor_state`,1-based)→ 250ms trailing-edge 防抖 →
  `Client::update_preview_context`(UDS) → `dozerd`
  (`crates/dozerd/src/preview_context.rs`)→ `dozer-mcp` 的
  `get_preview_context`。**注意**:这只是"editor/app → dozerd"一段,和
  webview 内部无关;Phase B 起要单独为这一段设计节流。

## 2. `editor / tabular / json_tree / loading / truncated` 合法组合与消费方

| 场景 | editor | tabular | json_tree | loading | truncated | 消费方 |
|---|---|---|---|---|---|---|
| `Blank` 占位 | None | None | None | false | false | 渲染空白卡;不进 webview 池 |
| 代码/配置/纯文本(异步) | None→Some | None | None | true→false | 视大小 | `is_native_editor_candidate`、`apply_native_load`、Find、保存、MCP context |
| JSON/JSON5(双视图) | Some | None | Some | false | false | 树/原文切换、webview 池排除 |
| JSONL/NDJSON | None | None | Some | false | false | 树/原文切换、webview 池排除(注意 editor 为 None 也**不**进池) |
| CSV/TSV/XLSX | None | Some | None | false | false | Tabular Viewer、`pending_tabular_loads` |
| Markdown/HTML、图片/PDF/媒体、压缩包、未知 | None | None | None | false | false | wry/flyfish;`wry_toggle_eligible` 决定是否有切码按钮 |
| 只读大文件(整读/分块) | Some(只读) | None | None | false | 分块档 true | 大文件搜索条、`preview_load_more`、`read_more_bytes` |

不变式(改动前代码依赖):
- "是否进 webview 池" = `editor.is_none() && tabular.is_none() &&
  json_tree.is_none() && !loading && File`,出现在 `desired_webviews` /
  `active_webview_id` / `select` / `reload_all_webviews_for_theme` 四处。
- JSON 是**双视图**(editor + json_tree 同时有值),所以"有 json_tree 就不进
  池"这条必须与 editor 判据并存。
- `loading` 为真时三个 viewer 均 None,但**不**进池(等读盘结果)。

Phase A 已把上述四处判据收敛到 `PreviewTab::hosts_webview()`(backend 主判据
+ 旧字段兜底子句,保证读盘失败时仍退回 webview,行为不变)。

## 3. 路由 fixture(表驱动,测试运行时生成)

`preview/router.rs` 与 `preview/backend.rs` 的 `#[cfg(test)]` 用
`classify_preview` / `PreviewBackend::from_route` 覆盖:

源码、无扩展名文本(`.gitignore`)、Markdown/HTML、严格 JSON、
JSONC/JSON5、JSONL/NDJSON、CSV/TSV/XLSX、图片(PNG/SVG)、PDF、压缩包、
未知二进制、未知文本、空文件、无扩展名 `Makefile`/`LICENSE`。

`preview/file_profile.rs` 的测试覆盖:空文件、短文本、UTF-8 BOM、CRLF、
非法 UTF-8、含 NUL 二进制、扩展名伪装(`.txt` 里塞 NUL)、超长单行、
>64KiB 大文件、UTF-16 BOM。

## 4. 性能 fixture 生成器

不向仓库提交任何大二进制。性能/画像测试一律**运行时生成**:
- 画像:>64KiB 文本、超长单行(`file_profile.rs` 测试内联构造);
- 后续 Phase C 的 10/30/64/128/500MiB、50 万短行、1/5MiB 单行、JSONL、
  宽 CSV 由 `large_text`/`file_policy` 的测试用临时文件生成(Phase C 落地)。

## 5. 基线测试命令与已知失败

- 基线命令:`cargo test -p dozer-app`、`cargo clippy -p dozer-app
  --all-targets`、`cargo fmt --check`。
- **已知失败(改动前即存在,非本迁移引入)**:
  `workspace::tests::agent_icon_maps_each_kind_to_brand_icon`
  (`crates/dozer-app/src/workspace/tests.rs:719`)。`f10834b feat(agents): add
  brand icons for Aider, Codex, Goose` 加了 Codex 品牌图标,但该测试仍断言
  `agent_icon(Codex) == Bot`。本阶段不修,避免把旧失败误判成新回归。

## 6. Phase A 的刻意取舍(与规格目标形态的差异,留待后续 phase)

为满足"用户可见行为与迁移前一致",Phase A 的路由**逐项对齐改动前行为**,
以下规格目标形态**推迟**:

| 规格目标 | Phase A 现状 | 目标 phase |
|---|---|---|
| 未知 UTF-8 文本 → Code | 未知文本仍走 Flyfish 兜底 | Phase 4 |
| 未知二进制 → External/Unsupported | `Unsupported` 描述但仍是 Flyfish webview 兜底 | Phase D |
| 压缩包 → 外部打开 | `External` 描述但仍是 Flyfish webview 兜底 | Phase D |
| `Makefile`/`Dockerfile`/`LICENSE` → Code | 与旧行为一致仍走 Flyfish | Phase 4 |
| SVG → 图像 + CodeMirror XML | `Rendered`(Flyfish 图像),暂无源码切换 | Phase 4 |

`PreviewBackend::hosts_webview()` 对 `External`/`Unsupported` 返回 `true`
正是这份兜底的编码;Phase D 落地正式 fallback 后改为 `false` 并删除对应
adapter。
