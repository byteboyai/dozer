# 文件预览 Phase B 进度(CodeMirror host)

> 对应计划:`docs/superpowers/plans/2026-09-22-codemirror-agent-editor.md`。
> 本文件记录本轮已落地的部分、刻意未接线的部分,以及后续接线的具体步骤。
> 2026-09-22。

## 已完成

### Task 1:离线前端工程(主体)
- `crates/dozer-app/web/editor/` —— TypeScript + CodeMirror 6 前端包:
  - 依赖锁定(见 `package.json` / `package-lock.json`):`@codemirror/{state,
    view,commands,search,language}` + 首批语言包(rust/python/js/ts/json/
    markdown/html/css)+ `@lezer/highlight`。
  - 构建 `build.mjs`(esbuild):输出 `editor.js`(iife、minify、无 source
    map、无 legal comments)、`editor.css`、`index.html`、`fonts/JetBrainsMono.ttf`
    到 `crates/dozer-app/assets/editor/`(已提交)。产物无 CDN、无运行时
    Node、无本机绝对路径。
  - 主题 `themes.ts`:ByteBoy2077 深/浅两套,语法色锚定终端 16 色角色;
    字体 JetBrains Mono(打包 ttf)+ 系统 CJK fallback。
  - 基础编辑器:行号、折叠 gutter、搜索/替换、选区、history、read-only、
    括号匹配;**未开启**补全/LSP/lint/multicursor/minimap。
  - 单测 `protocol.test.ts`(`node --test`,Node 24 类型擦除)+ `tsc --noEmit`
    类型检查,均通过。
- `.gitignore` 忽略 `web/editor/node_modules/`(产物提交,依赖不提交)。

### Task 3:通用消息 envelope(主体)
- `crates/dozer-app/src/preview/webview_protocol.rs` —— 所有 webview host 共用
  的 `WebviewEnvelope<T>` + `EditorEvent`(ready/selection_changed/
  document_changed/save_requested/focus_changed/viewport_changed/view_state/
  failed)+ `EditorCommand`(set_document/reveal_position/select_range/
  replace_range/open_find/set_read_only/focus/serialize_view_state)。
  - `parse_event` 过大/非法 JSON/未知 kind 一律返回错误,不 panic。
  - `HostBinding::validate` 校验 version/project/panel/tab/document(JS 不能
    自报归属)。
  - `encode_command` 供 Rust 经 `evaluate_script` 下发。
  - 前端 `src/protocol.ts` 是同一契约的 TS 端,含镜像单测。

### Task 2:scheme / CSP / host 描述(部分)
- `crates/dozer-app/src/assets.rs`:`dozer://editor/` 命名空间——`index.html`/
  `editor.js`/`editor.css`/`fonts/*` 从 editor 根(与 flyfish 根同级的
  `editor/`,dev 与打包态同构)服务;`dozer://editor/__file__<abs>` 复用
  `allowed_files` 白名单(**沿用旧的 `assets_root` 参数,editor 根由其兄弟
  目录派生,未改 `handle_protocol` 签名**)。路径穿越(含百分号编码形态)拒绝。
- `crates/dozer-app/web/editor/src/index.html`:严格 CSP(`default-src 'none'`,
  脚本/字体仅 self,`connect-src 'self'` 供 `__file__` fetch,`style-src`
  因 CodeMirror 注入内联样式需 `'unsafe-inline'`)。
- `crates/dozer-app/src/preview/code_host.rs`:`EditorHostBinding`
  `(project_id, panel, tab_id, path)` → 稳定 `document_id` / webview 池 key
  (project 面板用 `PROJECT_PREVIEW_ID_OFFSET`)/ editor URL(逐段编码路径 +
  theme/ro/lang);`panel_and_tab_from_webview_id` 反解;`is_editor_url`;
  `codemirror_enabled()`(Cargo feature `codemirror`,默认关闭)。
- `crates/dozer-app/Cargo.toml`:`[features] codemirror = []`(开发开关,
  默认关闭 → 用户可见行为不变)。

## 刻意未接线(下一轮)

运行时的深度接线会改到多个**热点共享文件**(`preview/view.rs`、
`preview/state.rs`、`app/update.rs`、`runtime.rs`、`platform/window_events.rs`),
本轮为保证不与其他并行改动冲突、并保持树常绿,未落地。具体步骤:

1. **pane 绑定元数据**:给 `PreviewPane` 加 `project_id: Option<i64>` 与
   `panel: PanelKind`,`Workspace::from_restore` 注入(Files/Project 各一份)。
2. **路由开关**:`is_native_editor_candidate` 与 `push_tab` 的 native_load 分支
   在 `codemirror_enabled()` 时跳过老 iced editor(`PreviewKind::Code`)。
3. **desired_webviews**:`backend.kind()==Code && codemirror_enabled()` 的文件 tab
   产出 `EditorHostBinding::url(...)` 的 `WebviewSpec`(`hosts_webview` 在
   `editor.is_none()` 时已为真,无需改)。
4. **runtime 分流**:`sync_webview_pool` 按 `is_editor_url(&spec.url)` 走不同
   builder:editor host 不注入 flyfish 脚本,IPC 只把 body 作为
   `Message::EditorIpc(webview_id, body)` 回传。
5. **IPC 派发**:`App::update` 处理 `EditorIpc`:`parse_event` → 反解
   `(panel, tab_id)` → 按 tab 建 `HostBinding` → `validate`;`SaveRequested`
   走原子保存(同目录临时文件 + rename,保持 BOM/换行),清 dirty;
   `SelectionChanged`/`ViewportChanged` 节流后推 `preview_context`;
   `Failed` 落 `BackendState::Failed` 并给外部打开 fallback。
6. **Rust→编辑器**:`Message::EditorCommand { panel, tab_id, command }` →
   在事件环里 `evaluate_script("window.__dozer.dispatch(<envelope>)")`。
7. **Task 5/6/7**:折叠/reveal/select 全链路、`PreviewContext` 四 crate 扩展、
   路由切换验收。

## 验证

- `cargo check -p dozer-app --all-targets`、`--features codemirror`:通过。
- `cargo test -p dozer-app`:1185 passed,唯一失败为改动前既有的
  `agent_icon_maps_each_kind_to_brand_icon`。
- `cargo fmt --check`、`cargo clippy -p dozer-app --all-targets`:仅剩既有
  `file_history.rs` warning。
- 前端:`npm run typecheck`、`npm test`(6 passed)、`npm run build` 均通过。
