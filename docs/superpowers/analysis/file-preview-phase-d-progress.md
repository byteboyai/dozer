# 文件预览 Phase D 进度(Viewer 收敛与旧实现退役)

> 对应计划:`docs/superpowers/plans/2026-09-22-preview-viewer-consolidation.md`。
> 2026-09-22。

## 已完成(本轮,additive、不删除任何旧实现)

### Task 1(部分):Rendered 的 Source 模式接入 CodeMirror host
- `PreviewTab::uses_editor_host()` 现覆盖 `Rendered { mode: Source }`:
  feature 打开时,Markdown/HTML 切到源码模式由 **CodeMirror editor host** 承载
  (替代老 iced `CodeView`),`hosts_webview()` 对该状态返回 false(Flyfish 让位),
  `desired_editor_webviews()` 为 Source 产出 `lang=markdown|html` 的 editor spec。
- `enter_code_mode()` 在 feature 下**不再构造 iced editor**,只翻转 backend mode;
  `exit_code_mode()` 翻回 Rendered。`preview_pane_toggle_render_mode` 的
  "当前是否源码态"判据加入 `uses_rendered_source_editor()`,修掉 feature 下
  Source 无法切回 Rendered 的 toggle bug。
- `push_shell_tab` 恢复时把 Rendered 的持久 `Source` 模式落到 backend。
- 测试:Markdown 切 Source → editor host + `lang=markdown`、不吃 Flyfish。
- 默认(无 feature)行为不变,仍走老 iced 源码视图。

## 未完成(受 GUI 验收与"迁移前不删除"原则约束,需人工确认后再做)

1. **Task 2 剩余**:editor/rendered/json WebView 的 macOS 上下文菜单走 NSMenu
   (现用 WKWebView 系统原生菜单,见 Phase B 文档的取舍说明);"使用系统默认
   应用打开"已具备(Phase C 的错误横幅按钮),但未知/损坏/加密格式的**正式
   fallback 页面**未做。
2. **Task 3 `vanilla-jsoneditor`** 接入(Tree/Text、path select、revision 复用
   通用 envelope)——需要新前端包与 `dozer://json-editor/` host。
3. **Task 4 Streamed JSON** 抽取与重构。
4. **Task 5 删除自研普通 JSON tree**、**Task 7 删除老 iced `CodeView`**:按总规格
   "旧实现只有在替代能力通过验收后才删除"。二者都要求 `codemirror` feature
   转默认开启并在真机完成人工验收,当前 feature 仍默认关闭,故**不删除**。
5. **Task 6 Tabular 接入统一 backend**:路由/backend 描述已具备(Task A),
   Agent 上下文与资源登记待补。
6. **Task 8** 最终路由矩阵全量验收 + 旧 spec 状态更新。

## 验证

- `cargo check/build -p dozer-app --all-targets`(默认与 `--features codemirror`):
  通过。
- `cargo test -p dozer-app`:默认 **1254 passed / 0 failed**、feature **1238
  passed / 0 failed**(另有 1 ignored)。
- `cargo fmt --check`、`cargo clippy` 干净(仅既有 `file_history.rs` warning)。
- 前端未改动(无需重建)。
