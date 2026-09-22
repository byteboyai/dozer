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

### Task 5(第一步):普通 JSON/JSONL 的 Text 模式接入 CodeMirror
按计划"先让普通 JSON Text 使用 CodeMirror JSON language;自研 Tree 暂不删除":
- `uses_editor_host()` 覆盖 `Json`/`Streamed`;feature 下 JSON/JSONL 的 **Text**
  模式由 editor host(`lang=json`)承载,**Tree/Streamed 视图仍走原生
  `json_tree`**(不删除)。
- `is_native_editor_candidate` 与 `push_tab` 在 feature 下不再为 JSON 构造 iced
  editor;`load_preview_tab` 的 JSON/Streamed 分支改为挂 `json_tree` 后台加载 +
  editor host Ready。
- `desired_editor_webviews` 仅在 JSON 处于 Text 模式时产出 editor spec(Tree 模式
  不产出,由原生树渲染)。
- `extension_to_syntax` 增加 `jsonl/ndjson → json`,让 JSONL 文本视图也有 JSON 高亮。
- 测试:JSON Text 模式产出 `lang=json` spec;3 个旧语义测试按 feature 门控。

## 未完成(受 GUI 验收与"迁移前不删除"原则约束)

1. **Task 2 剩余**:NSMenu 上下文菜单(取舍见 Phase B 文档);Unsupported/损坏/
   加密格式的正式 fallback 页面。
2. **Task 3 `vanilla-jsoneditor`** 接入;Task 4 Streamed JSON 重构。
3. **Task 5 其余 / Task 7 删除**自研普通 JSON Tree 与老 iced `CodeView`:要求
   `codemirror` 转默认开启 + 真机全量验收。当前 feature 仍默认关闭,故**不删除**。
4. **Task 6 Tabular**:路由/backend 已具备;CSV/TSV 原文 CodeMirror 模式、Agent
   sheet/cell 上下文、sheet/滚动锚点持久化待补。
5. **Task 8** 路由矩阵全量验收 + 旧 spec 状态更新。

## 验证

- `cargo check/build -p dozer-app --all-targets`(默认与 `--features codemirror`):
  通过。
- `cargo test -p dozer-app`:默认 **1254 passed / 0 failed**、feature **1235
  passed / 0 failed**(另有 1 ignored)。
- `cargo fmt --check`、`cargo clippy` 干净(仅既有 `file_history.rs` warning)。
