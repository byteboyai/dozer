# PlantUML 文件预览:安全回归与端到端验收记录

> 对应计划:`docs/superpowers/plans/2026-10-04-plantuml-file-preview.md` Task 9。
> 设计:`docs/superpowers/specs/2026-10-04-plantuml-file-preview-design.md` §9。
>
> 本文档记录 Task 9 的证据:已完成的自动化门禁与安全回归结果,以及需要人工在
> 发布包 / 真实 WKWebView / 断网环境下逐项确认的验收矩阵。

## 0. 环境与版本

| 项 | 值 |
|---|---|
| 采集日期 | 2026-10-05 |
| 机器 | _(人工验收时填写:macOS 版本 / 机型)_ |
| 提交 | Task 8 = `71c9bd9f`;Task 9 见本次提交 |
| 引擎 | `@plantuml/core@1.2026.8`(MIT),`sha512-md2wGuaIJnAq1yKkedSpQE68cLsvZvctwzWHkDAnC3DTVLe/MSd/2vr4ETwr6jtc1av8cdxCohFuW4VGejMHyw==` |
| 运行时依赖 | 无 Java、无 Node(引擎为 TeaVM JS 产物) |

### 打包资产与体积(`crates/dozer-app/assets/plantuml-viewer/`)

| 文件 | 体积 | 说明 |
|---|---|---|
| `plantuml.js` | 3.9 MiB | 官方引擎 ESM 入口(逐字节复制,未过 esbuild) |
| `viz-global.js` | 1.4 MiB | Graphviz/Viz.js classic 全局(须先于引擎加载) |
| `emoji.js` | 1.9 MiB | 引擎运行时资源 |
| `themes.js` | 326 KiB | 引擎主题 |
| `openiconic.js` | 51 KiB | 图标资源 |
| `stdlib/c4.min.js` | — | C4 stdlib(注入 `PLANTUML_STDLIB`) |
| `bundle.js` | 12 KiB | 我方薄胶水(加载/消息/缩放/错误映射) |
| `bundle.css` + `index.html` | — | 工具栏 + 舞台 + 严格 CSP |

## 1. 已完成的自动化门禁(2026-10-05)

| 门禁 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | ✅ 通过 |
| 日志来源 | `scripts/check-log-scope.sh` | ✅ `log scope check: ok` |
| 面板边界 | `python3 scripts/audit/check_panel_boundary.py` | ✅ `ok (122 refs in 5 files, baseline ratchet)` |
| Clippy | `cargo clippy -p dozer-app --all-targets` | ✅ 改动文件零告警(仅存量第三方 `block v0.1.6` future-incompat 提示) |
| Rust 测试 | `cargo test -p dozer-app` | ✅ 1851 passed;3 failed(**均为改动前既有失败**,见下) |
| 引擎离线条带 | `node scan-offline.mjs` | ✅ `offline asset scan passed (9 files)` |
| 引擎冒烟 | `npm test`(build + render-smoke + sanitize) | ✅ `all smoke checks passed` |

**既有失败(非本特性引入,与基线一致)**:`extensions::files::tests::
delete_confirm_spec_reflects_pending_target`(NFD/NFC 文件名)、
`extensions::git_log::tests::build_marks_head_branch_and_labels`、
`extensions::git_log::tests::build_populates_time_and_is_merge`(依赖当次真实仓库
状态)。另有并行顺序偶发 `assets::tests::serves_vendored_asset_with_mime`。

## 2. 安全回归证据(自动化,对应计划第 465–466 行)

| 风险 | 防护 | 覆盖测试 |
|---|---|---|
| 恶意 SVG 执行脚本 | 前端 sanitizer 去 `<script>`/`foreignObject`/`on*`/外部 `href`;超限拒绝 | `web/plantuml-viewer/src/sanitize.test.mjs`(6 用例) |
| 畸形 / 缺字段 / 超大 envelope | 严格反序列化 + 长度门禁 | `webview_protocol::tests::plantuml_event_rejects_oversized_unknown_and_malformed`、`plantuml_events_accept_ready_rendered_and_open_source` |
| 超大错误字符串 / 越界行号 | 错误截断到 `MAX_PLANTUML_ERROR_CHARS`、行号钳制到 `MAX_PLANTUML_LINE` | `plantuml_event_truncates_error_and_clamps_line` |
| binding 不匹配 / 迟到事件 | `HostBinding::validate` + `carries_terminal_result`/`is_stale` | `parses_and_validates_plantuml_events`、`plantuml_stale_terminal_results_are_droppable`、`apply_plantuml_event_ignores_non_plantuml_tab`、`apply_plantuml_event_rendered_stale_revision_is_dropped` |
| include bomb(数量/总量/深度) | 深度 16、文件 128、总 16 MiB、根 4 MiB 上限 | `rejects_too_deep_chain`、`rejects_too_many_files`、`rejects_total_bytes_over_limit`、`rejects_oversized_root` |
| include 环 | 环检测 | `detects_include_cycle` |
| 目录穿越 / 绝对路径 / `file://` | 拒绝非项目内路径 | `rejects_parent_dir_escape_lexically`、`rejects_file_url_and_absolute_and_dynamic` |
| symlink 越界 | canonicalize 后校验仍在根内 | `rejects_symlink_escape_even_when_text_target_is_in_root` |
| 远程 include / `!includeurl` | 明确拒绝 | `rejects_includeurl_and_plain_urls` |
| 非 UTF-8 / 目录 / 非规则文件 | 拒绝 | `rejects_non_utf8_root`、`rejects_non_utf8_include`、`rejects_directory_include` |
| 二进制伪装成 `.puml` | 内容安全检查优先于扩展名 | `plantuml_binary_masquerade_does_not_enter_renderer` |
| 资源耗尽(重型 WebView) | 计入 `max_heavy_webviews` + 预览总预算;拒绝后进 `Failed` 不再重建 | `plantuml_rendered_host_carries_reserve_hint_in_desired_webviews`、`diagnostics_distinguishes_plantuml_via_estimate_cost` |
| host 静态资源越界 | `dozer://plantuml-viewer/` 仅服务 vendored 资产 | `plantuml_viewer_rejects_traversal_and_has_no_file_endpoint`、`plantuml_viewer_serves_vendored_files` |
| CSP 无网络 | `default-src 'none'`,无 `connect-src` | `assets.rs::plantuml_viewer_host_has_strict_csp_and_no_external_refs` |
| 产物残留绝对路径 / sourcemap | build 门禁 | `plantuml_viewer_bundle_has_no_absolute_paths_or_sourcemap` |
| 离线条带(无公网/`file://`/`eval`) | `scan-offline.mjs` | `offline asset scan passed (9 files)` |
| 零网络请求(引擎路径) | harness 阻断 XHR/fetch 并断言零次 | `render-smoke.mjs` → `zero network attempts` |

## 3. 安全回归(Rust 侧协议 / 生命周期 / include)

上述 §2 覆盖以下 spec §9.1 条目:router 表(五种扩展名/大小写/空文件/二进制伪装/
持久化 mode)、backend(`RenderedRenderer::PlantUml`、Source↔Rendered)、protocol
(完整/缺字段/畸形/超大/binding 不匹配/旧 revision)、include(正常/嵌套/once/环/
穿越/绝对/symlink/远程/深度/数量/总字节)、assets(host/bundle/stdlib 存在、CSP
无网络、未知路径 404)、lifecycle(Files/Project、tab 关闭、项目关闭、reload、
suspend/resume、迟到事件)。

## 4. 人工验收矩阵(需在发布包 / 断网环境 / 真实 WKWebView 执行)

> 以下各项由人工在 `cargo tauri`/`cargo run -p dozer-app` 或打包后的 `.app` 上
> 执行;执行时逐项记录结果与截图路径。**这是 Task 9 尚未自动化的部分。**

- [ ] **A1** Files 与 Project 分别打开同一 `.puml`,均默认显示图形。(spec §9.2.1)
- [ ] **A2** 图形↔源码往返;源码可编辑保存,保存后图形刷新且无旧 revision 闪回。(§9.2.2)
- [ ] **A3** 适配窗口 / 100% / 放大 / 缩小 / 重置视图,在深、浅主题下均正常。(§9.2.3)
- [ ] **A4** 五种扩展名 + C4 fixture 在**断网**下全部渲染。(§9.2.4)
- [ ] **A5** 语法错误显示原因;点击"查看源码"定位到错误行(引擎提供行号时)。(§9.2.5)
- [ ] **A6** 项目内多层 include 正常;修改被 include 文件后已打开的图自动更新。(§9.2.6)
- [ ] **A7** `!includeurl` / 目录穿越 / symlink 越界被明确拒绝;抓包(或网络禁用)确认**零请求**。(§9.2.7)
- [ ] **A8** 连续快速切换三个大图,最终只显示当前文件,UI 不冻结。(§9.2.8)
- [ ] **A9** 达到 WebView 预算后能 suspend/resume,不反复销毁重建。(§9.2.9)
- [ ] **A10** 无 Java、无 Node、断网的发布包可完成上述操作。(§9.2.10)
- [ ] **A11** 窗口缩放 / 左右栏拖动 / 面板最大化 / Files↔Project 镜像布局均正常。(计划第 469 行)
- [ ] **A12** 快速切 tab / 保存 / include 变化时 revision 不乱序。(计划第 467 行)
- [ ] **A13** 打开足够多重型 preview 触发预算,验证 suspend/resume 与 RSS 回落。(计划第 468 行)

### 抓包 / 断网方法(供执行者参考)

- 断网:m 关 Wi-Fi,或用 `sudo pfctl` / Little Snitch 阻断 `dozer` 进程出站。
- 抓包:`tcpdump -i any -n host not 127.0.0.1 and not ::1` 观察渲染期间是否出现外联。
- 关键断言:打开并渲染 `.puml`、含 `!include <C4/...>`、含被拒的 `!includeurl`
  fixture 三种情况,出站流量均为零。

## 5. 结论

- 自动化部分(Task 9 第 462–466、470–471 行的可机验项)已完成并全部通过。
- 人工验收矩阵(§4 A1–A13)待人工在发布包/断网/真实 WKWebView 上执行并回填。
  在此之前的发布应保留 Source mode 作为稳定退路(spec §10)。
